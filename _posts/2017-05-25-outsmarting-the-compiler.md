---
layout: post
title: "Outsmarting the compiler"
description: "Can you?"
category:
tags: []
---

Suppose we have two very similar structs which we need to partially populate "ahead of time" and store somewhere.
Then, a bit later, we need to **very quickly** finish populating the structs.
Here are some example structs:

```c++
struct __attribute__((packed)) A {
  int64_t a;
  int64_t b;
  char    arr[PADDING1];
  int64_t c;
};

struct __attribute__((packed)) B {
  int64_t a;
  int64_t b;
  char    arr[PADDING2];
  int64_t c;
};
```

The "padding" arrays are populated ahead of time, so we just need to set `a`, `b`, and `c` for each struct (quickly):

```c++
template <typename T>
void writeFields(T* t)
{
  t->a = 12;
  t->b = 25;
  t->c = 16;
}
```

Unfortunately, we don't statically know what struct we are going to have to operate on; we only get this information at runtime.
We just have a blob of memory and a tag which indicates which of the two variants of the struct is sitting in the blob of memory:

```c++
enum class Variant { eA, eB };

struct Wrapper {
  Variant v;
  char payload[];
};
```

So, our fast path `write` function will need to take a wrapper struct, switch on the tag, then call the appropriate version of `writeFields`:

```c++
void write(Wrapper* w)
{
  if (w->v == Variant::eA) {
    writeFields<A>(reinterpret_cast<A*>(w->payload));
  }
  else {
    writeFields<B>(reinterpret_cast<B*>(w->payload));
  }
}
```

If `PADDING1 == PADDING2`, then, regardless of the value of the tag (which struct we are populating), we will need to write to the same offsets.
The cast and the templated function call will all compile out.
Take a look (`clang-4.0 --std=c++1z -O3`):

```nasm
.LCPI2_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
write(Wrapper*):                      # @write(Wrapper*)
        movaps  xmm0, xmmword ptr [rip + .LCPI2_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 4], xmm0
        mov     qword ptr [rdi + 36], 16
        ret
```

Before we move on, take a moment to appreciate what your compiler just did for you:
1. It allowed you to write a type safe `writeFields` method. If the layout of the struct changes for some reason, this part of the code will not begin to misbehave.
1. It removed the cost of the abstraction when it could figure out how to.

Unfortunately, if `PADDING1 != PADDING2`, we will need to write the value of `c` in a different location in struct `A` and struct `B`.
In this case, it looks like we will need read the tag out of the `Wrapper*`, then branch to the appropriate `writeFields` method.
We are good programmers, we know that branches might be expensive, so we really want avoid any branching.

We can skip the branch by storing the offset in our wrapper struct and precomputing the offset when the wrapper is set up.
Introduce a new wrapper type (and abandon all type safety):

```c++
struct WrapperWithOffset {
  Variant v;
  size_t offset;
  char payload[];
};
```

Next, we can write a new function which will operate on structs of type `A` or type `B`, but, instead of writing to `c` directly, it computes a pointer to `c` using the offset we've stored in the wrapper, then writes to that pointer.

```c++
void writeFieldsWithOffset(A* t, size_t c_offset)
{
  // make sure a and b are always at the same offset in struct A and struct B
  static_assert(offsetof(A, a) == offsetof(B, a), "!");
  static_assert(offsetof(A, b) == offsetof(B, b), "!");

  t->a = 12;
  t->b = 25;

  // c will be at the offset we've provided
  *(int64_t*)(((char*)t + c_offset)) = 16;
}

void writeLessSafe(WrapperWithOffset* w)
{
  A* a = reinterpret_cast<A*>(w->payload);
  writeFieldsWithOffset(a, w->offset);
}
```

Checking the code, this compiles down to exactly what we were hoping it would (again with clang-4.0)!

```nasm
.LCPI1_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
writeLessSafe(WrapperWithOffset*): # @writeLessSafe(WrapperWithOffset*)
        mov     rax, qword ptr [rdi + 8]
        movaps  xmm0, xmmword ptr [rip + .LCPI1_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 16], xmm0
        mov     qword ptr [rdi + rax + 16], 16
        ret
```

Hooray, no conditional generated, exactly as we desired.
We've outsmarted the compiler!

#### Assertion Failed: smarter_than_compiler

Let's set `PADDING1 = 16` and `PADDING2 = 17`.
The code generated on clang-4.0 for `write(Wrapper*)` looks quite interesting:

```nasm
.LCPI2_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
write(Wrapper*):                      # @write(Wrapper*)
        xor     eax, eax
        cmp     dword ptr [rdi], 0
        movaps  xmm0, xmmword ptr [rip + .LCPI2_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 4], xmm0
        setne   al
        mov     qword ptr [rdi + rax + 36], 16
        ret
```

This code is still very slightly longer than the unsafe code written previously, but, its really not bad at all.

The compiler has succeeded in avoiding a branch using a rather clever `cmp` and `setne` instruction pair.
Essentially, clang figured out that it could compute the offset of `c` using the tag we've placed in the `Wrapper`'s `Variant` field.
In this case, I've allowed the enum values to default to `0` and `1` (hence the `cmp dword ptr [rdi], 0` checking if the first thing in the functions first arg is equal to 0).

What happens if we change the values?

```c++
enum class Variant { eA = 666, eB = 1337 };
```

```nasm
.LCPI2_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
write(Wrapper*):                      # @write(Wrapper*)
        mov     eax, dword ptr [rdi]
        movaps  xmm0, xmmword ptr [rip + .LCPI2_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 4], xmm0
        xor     ecx, ecx
        cmp     eax, 666
        setne   cl
        mov     qword ptr [rdi + rcx + 36], 16
        ret
```

The code has changed slightly to account for the new potential values of `Wrapper::v`, but it looks much nicer than a branch.

### Meaner PADDING
Reminder: In the previous examples `PADDING1 = 16` and `PADDING2 = 17`.
What happens to the generated code if we make the paddings completely wacky?

With `PADDING1 = 16` and `PADDING2 = 173`, and with the enum values reverted to their defaults:

```nasm
.LCPI1_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
writeLessSafe(WrapperWithOffset*): # @writeLessSafe(WrapperWithOffset*)
        mov     rax, qword ptr [rdi + 8]
        movaps  xmm0, xmmword ptr [rip + .LCPI1_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 16], xmm0
        mov     qword ptr [rdi + rax + 16], 16
        ret

.LCPI2_0:
        .quad   12                      # 0xc
        .quad   25                      # 0x19
write(Wrapper*):                      # @write(Wrapper*)
        cmp     dword ptr [rdi], 0
        movaps  xmm0, xmmword ptr [rip + .LCPI2_0] # xmm0 = [12,25]
        movups  xmmword ptr [rdi + 4], xmm0
        mov     eax, 32
        mov     ecx, 189
        cmove   rcx, rax
        mov     qword ptr [rdi + rcx + 4], 16
        ret
```

`writeLessSafe` doesn't change, as expected.
`write` does get tweaked a bit to account for the new offsets, but its still pretty great code.

So, have we beaten the compiler?
The answer to that depends on which compiler you ask.

### gcc 7.1 (--std=c++1z -O3)
#### `PADDING1` == `PADDING2`

```c++
writeLessSafe(WrapperWithOffset*):
        mov     rax, QWORD PTR [rdi+8]
        mov     QWORD PTR [rdi+16], 12
        mov     QWORD PTR [rdi+24], 25
        mov     QWORD PTR [rdi+16+rax], 16
        ret
write(Wrapper*):
        mov     eax, DWORD PTR [rdi]
        mov     QWORD PTR [rdi+4], 12
        mov     QWORD PTR [rdi+12], 25
        mov     QWORD PTR [rdi+36], 16
        test    eax, eax
        je      .L7
        rep ret
.L7:
        rep ret
```

That's a little odd.

#### `PADDING1 = 16` and `PADDING2 = 17`

```c++
write(Wrapper*):
        mov     eax, DWORD PTR [rdi]
        mov     QWORD PTR [rdi+4], 12
        mov     QWORD PTR [rdi+12], 25
        test    eax, eax
        je      .L7
        mov     QWORD PTR [rdi+37], 16
        ret
.L7:
        mov     QWORD PTR [rdi+36], 16
        ret
```

#### `PADDING1 = 16` and `PADDING2 = 173`

```c++
write(Wrapper*):
        mov     eax, DWORD PTR [rdi]
        mov     QWORD PTR [rdi+4], 12
        mov     QWORD PTR [rdi+12], 25
        test    eax, eax
        je      .L7
        mov     QWORD PTR [rdi+193], 16
        ret
.L7:
        mov     QWORD PTR [rdi+36], 16
        ret
```

Interesting.
This branch felt *almost* detectable in some micro-benchmarks, but I would require additional testing before I'm willing to declare that it is harmful.
At the moment I'm not convinced that it hurts much.

### Conclusion
No conclusion.
None of my benchmarks have managed to detect any convincing cost for this branch (even when variants are randomly chosen inside of a loop in an attempt to confuse branch predictor) so none of this actually matters (probably).
The only interesting fact my benchmarks showed is that clang 4.0 looked very very slightly faster than gcc 6.3, possibly because of the vector instructions clang is generating, but also possibly because benchmarking is hard and I'm not benchmarking on isolated cores.
Here's some code: [gist](https://gist.github.com/dpzmick/a8f937c5e35185092b6af9a5ed87a7b8).
