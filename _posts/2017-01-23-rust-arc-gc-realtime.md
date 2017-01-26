---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on a synthesizer (the kind that makes sounds) in Rust.
This post is the second post in a series of posts about this project.
See my [blog series](/series/) page for links to other posts.

If you don't know anything at all about realtime audio programming, you might want to read the first post in this series, [Audio Programming 101](/2016/12/17/audio-basics/), or watch [this talk](https://www.youtube.com/watch?v=SJXGSJ6Zoro) from the Audio Developers Conference, to get a little bit of background.

In short, there's a realtime thread that can never be blocked in any way.
The realtime thread is responsible for sending all of the audio which an application will produce to an audio system, at exactly the right moments.
If the realtime thread ever fails to generate the audio it needs to generate, bad things happen.
That means locks, I/O, allocation are all off limits in the realtime thread.

Sending messages from non-realtime threads to the realtime thread is trickier than it might be in a "normal" application because we can't do these things.
There are many, many techniques which can be used to work around this trickiness.
This post is a discussion of one such method (presented in [this cppcon talk](https://www.youtube.com/watch?v=boPEO2auJj4)) implemented in [Rust](https://www.rust-lang.org/en-US/).

# Messaging between threads
Suppose we are developing a synthesizer which produces sounds when keys are pressed on a [MIDI keyboard](https://en.wikipedia.org/wiki/MIDI_controller#Keyboards).
The audio library calls a function we provide once ever 6 or so milliseconds to request a list of samples from us.
The library calls our function with 2 arguments: 1) How many samples it wants 2) what key presses we need to handle.
The callback function uses a precomputed list of samples to generate sounds every time it is called.
To modify the properties of the sounds that are produced, the user edits settings with a user interface.

It would be painful (and incorrect) to attempt to handle UI events in the realtime thread, so we will run a UI thread to handle the UI events.
Whenever the UI thread gets an event to handle, it needs to compute a new sample list, then send the list to the realtime thread.

Since we can't lock, let's use a queue to send some sort of message between threads.
The queue that we choose needs to have a few properties:

* Must be a [lock free queue](https://pdfs.semanticscholar.org/a909/1ef790788c5d252cad94dd6862adf457e073.pdf)
* Must be able to preallocate all of its nodes (cant't allocate or free memory for a node on a push or pull)

I want to place messages on the heap so that they do not need to be copied as we move them around.
If messages lives on the heap, we must ensure they are allocated and freed outside of the realtime thread (we can't call allocation functions on the realtime thread).

## Reference counted garbage collection
It is totally fine to allocate on the UI thread, so when the UI thread handles an event it will compute a new list of samples and stick them into a freshly allocated block of memory.
Then we will ship this message over to the realtime thread.

When the realtime thread takes ownership of the message, it will need to hold onto the data for some undefined period of time.
But, when the realtime thread is done with the message, it cannot free it.

To solve this, let's run one more thread to clean up messages which are no longer being used by the realtime thread.

Whenever the UI thread allocates space for a message using standard allocators, it will wrap the message in a [reference-counted pointer](https://doc.rust-lang.org/std/sync/struct.Arc.html).
It then will let the collector thread know it should start keeping an eye on the reference-counted pointer.
The collector will store the pointer in a list.
When the reference count falls to 1, the collector is the only thread with a reference, and it can safely free the memory.
The pointer is sent to the realtime thread, then, when the realtime thread drops the message, the reference count will drop.
Sometime later, the collector thread will observe the decreased reference count and free the message.

[Here](/img/sound/gc_queue.pdf) is a slideshow/animation demonstrating this process.

## Tradeoffs
Let's consider the theoretical behavior of this approach.
Note that anything I have to say should be taken with grain of salt; I haven't benchmarked anything, so I really have no evidence to support anything I'm claiming.

First, let's talk about when we would not want to use this approach.

If the realtime thread always consumes new messages in a predictable amount of time, we can preallocate a fixed size buffer to hold onto "in flight" UI messages.
When the UI needs to send a message it can grab one of the preallocated messages and use it.
Some predictable amount of time later, the message can be returned to the pool.

This is also a bad idea if the UI thread generates messages significantly faster than the realtime thread consumes them.
It might be fine for the realtime thread to lag behind the UI thread (if it eventually catches up), but the GC pointer list is going to get quite large.
If we do our GC scan frequently, we will be using a lot of cpu time scanning this list.
If we slow the collector down, the list is going to keep growing, and so will our memory usage.
In other words, its a sticky situation.
A modern computer can probably handle this load, but we should avoid generating more load than necessary so that other audio applications running at the same time can use as much time as they need.

Finally, if the realtime thread needs to send a message to the UI thread, it can't just allocate memory and toss it at the GC thread for cleanup later.
We could still use the GC+queue method discussed here to send messages to the realtime thread, but we probably only have time to build one good messaging system (we want to make audio, not send messages back and forth!)

If none of the above are true, a simple GC thread with some reference counted pointers might be a nice way to avoid adding lots of complexity to a small system.
It also saves us from the need for a custom allocation mechanism, lets us send messages of various and dynamic sizes, and frees us from the burden of strict capacity constraints.
So, if we don't need something more clever, maybe this is a good thing to try out.

Finally, since we are using reference counting to manage memory, there will be some runtime cost to increment and decrement the reference counts.
This isn't a big deal for us, in this case, because the performance is predictable (we won't be suddenly surprised by the non-deterministic reference count incrementing).

Regardless of the actual efficacy of this approach, it will be interesting to try to build one in Rust, so let's get started.

# Let's make one
For the sake of these examples, let's assume that the built in Rust [mpsc channel](https://doc.rust-lang.org/std/sync/mpsc/index.html) is an appropriate lock free queue.
It will be pretty easy to swap this with something different later, and, if we use the standard library, all of the examples will easily run in the Rust playground.
We are also going to fake a bunch of the details of the audio library.

## Fake audio library
[Rust Playground Link](https://play.rust-lang.org/?gist=27d1b7a693ffe01ac899b991317b170f&version=stable&backtrace=0).

We don't need to walk through this code, it just makes some threads and calls some empty functions.
The important bits are the `RealtimeThread::realtime_callback` function and the `UIThread::run` functions.
In this example, the realtime callback function says "I'm done!" to let the realtime thread shutdown, and the UI thread does nothing at all.

Here's the code:

```rust
use std::thread;

#[derive(PartialEq)]
enum CallbackStatus {
    Continue,
    Shutdown,
}

// "library" code starts here
type Samples = [f32; 64];

fn run_threads(mut rt: RealtimeThread, mut ui: UIThread) {
    let join_handle = thread::spawn(move || {
        println!("[ui] thread started");
        ui.run();
        println!("[ui] thread shutting down");
    });

    println!("[realtime] thread started");
    let mut output = [0.0; 64];
    while rt.realtime_callback(&mut output) != CallbackStatus::Shutdown { }
    println!("[realtime] thread shutting down");

    join_handle.join().unwrap();
}
// end of "library" code

/// A struct containing the realtime callback and all data owned by the realtime thread
struct RealtimeThread {
    // some members here eventually
}

impl RealtimeThread {
    fn new() -> Self { RealtimeThread{} }

    /// realtime callback, called to get the list of samples
    fn realtime_callback(&mut self, output_samples: &mut Samples) -> CallbackStatus {
        CallbackStatus::Shutdown
    }
}

/// A struct which runs the UI thread and contains all of the data owned by the UI thread
struct UIThread {
    // some members here eventually
}

impl UIThread {
    fn new() -> Self { UIThread{} }

    /// All of the UI thread code
    fn run(&mut self) {
        // do nothing!
    }
}

fn main() {
    let rt = RealtimeThread::new();
    let ui = UIThread::new();
    run_threads(rt, ui);
}
```

Output (one of many possible):

```
[realtime] thread started
[realtime] thread shutting down
[ui] thread started
[ui] thread shutting down
```


## Sending Arcs between threads
Now that we have an "audio library," let's try to make some messages and pass them between threads.

The `RealtimeThread` struct will need to hold on to a list of samples which it will use to populate the `output` samples every time the callback is called.
We want these samples to be heap allocated and reference counted, so we wrap them in an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html).
Finally, we want to leave the samples uninitialized until the UI thread sends us some, so we wrap the `Arc<Samples>` in an [`Option`](https://doc.rust-lang.org/std/option/enum.Option.html).

```rust
struct RealtimeThread {
  current_samples: Option<Arc<Samples>>,
}
```

Now that the realtime thread has a list of samples, we can fill in a bit of the body of the realtime callback function:

```rust
fn realtime_callback(&mut self, output_samples: &mut Samples) -> CallbackStatus {
    self.current_samples.as_ref().map(|samples| {
        // samples: &Arc<[f32; 64]>
        output_samples.copy_from_slice(samples.as_ref())
    });

    CallbackStatus::Continue
}
```

The function [`copy_from_slice`](https://doc.rust-lang.org/std/primitive.slice.html#method.copy_from_slice) will `memcpy` the samples we are holding onto into the buffer provided by the audio library.

Moving over to the UI thread, first, we need to be able to compute a list of samples to compute.
Here is a function that computes 64 samples along a sine wave with a given peak amplitude:

```rust
/// computes the samples needed for on cycle of a sine wave
/// the volume parameter sets the audible volume of sound produced
fn compute_samples(&self, volume: f32) -> Samples {
    assert!(volume >= 0.0);
    assert!(volume <= 1.0);

    // we need to populate 64 samples with 1 cycle of a sine wave (arbitrary choice)
    let constant_factor = (1.0/64.0) * 2.0 * f32::consts::PI;
    let mut samples = [0.0; 64];
    for i in 0..64 {
        samples[i] = (constant_factor * i as f32).sin() * volume;
    }

    samples
}
```

The UI thread will generate some fake events, and compute samples for these events:

```rust
/// All of the UI thread code
fn run(&mut self) {
    // create 5 "ui events"
    for i in 0..5 {
        let volume = i as f32 / 10.0;
        let samples = Arc::new(self.compute_samples(volume));

        // send the samples to the other thread
    }

    // tell the other thread to shutdown
}
```

Now that we've done all of that, we need to send the samples between threads.

## Message type

As discussed previously, we will create the `Arc` on the UI thread, then send it to the realtime thread.

```rust
enum Message {
    NewSamples(Arc<Samples>),
    Shutdown,
}
```

Remember when I said that we would make a bunch of assumptions about the `mpsc` queues?
Here's where I'm going to do that.
We are going to assume that this queue follows all the properties we need a realtime queue to follow.
For a quick reminder, those are:
* No locks
* No allocation (or deallocation) in the realtime thread.

To send messages between the threads, we will use [`mpcs::sync_channel`](https://doc.rust-lang.org/std/sync/mpsc/fn.sync_channel.html) to create a synchronous channel (queue).
This channel is bounded, so a sender cannot add a new message to the queue unless there is currently space available.
We are going to set the buffer size to zero.
From [the docs](https://doc.rust-lang.org/std/sync/mpsc/fn.sync_channel.html):

> Note that a buffer size of 0 is valid, in which case this [channel] becomes "rendezvous channel" where each send will not return until a recv is paired with it.

This "channel" will have two ends; one which can send messages and one which can receive messages.
Lets create both of them in the `main` method.
The send side will be called `tx` (for transmit) and the receive side is called `rx`.
Whenever a message is placed on `tx` it will become available on `rx`.

Then, we let each of our threads take ownership of the appropriate channel.
We give `rx` to the `RealtimeThread`, because it will receive messages, and `tx` to the `UIThread`, because it will be sending them.

```rust
fn main() {
    let (tx, rx) = mpsc::sync_channel(0);
    let rt = RealtimeThread::new(rx);
    let ui = UIThread::new(tx);
    run_threads(rt, ui);
}
```

Then, modify both thread structs and both `new` functions.

```rust
struct RealtimeThread {
    current_samples: Option<Arc<Samples>>,
    incoming:        mpsc::Receiver<Message>,
}

// ...

struct UIThread {
    outgoing: mpsc::SyncSender<Message>,
}

// changes to new omitted
```

Now, let's get our threads sending messages, starting with the UI thread.
If any sends fails, something has gone horribly wrong, so its fine to `unwrap` the result of these sends.

```rust
/// All of the UI thread code
fn run(&mut self) {
    // create 10 "ui events"
    for i in 0..10 {
        let volume = i as f32 / 10.0;
        let samples = Arc::new(self.compute_samples(volume));

        // send the samples to the other thread
        println!("[ui] sending new samples. Second sample: {}", samples[1]);
        self.outgoing.send(Message::NewSamples(samples)).unwrap();
    }

    // tell the other thread to shutdown
    self.outgoing.send(Message::Shutdown).unwrap();
}
```

In the realtime thread, we check if there is a new message on the queue.
If there is, handle it.
If not, just keep doing what we were doing.

```rust
/// realtime callback, called to get the list of samples
fn realtime_callback(&mut self, output_samples: &mut Samples) -> CallbackStatus {
    match self.incoming.try_recv() {
        // we've received a messaged
        Ok(message) => match message {
            Message::NewSamples(samples) => {
                println!("[realtime] received new samples. Second sample: {}", samples[1]);
                self.current_samples = Some(samples)
            },

            // If we got a shutdown message, shutdown the realtime thread
            Message::Shutdown => return CallbackStatus::Shutdown
        },

        // if we didn't receive anything, just keep sending samples
        Err(_) => ()
    }

    // copy our current samples into the output buffer
    self.current_samples.as_ref().map(|samples| {
        // samples: &Arc<[f32; 64>
        output_samples.copy_from_slice(samples.as_ref())
    });

    CallbackStatus::Continue
}
```

I've used a `println!` here only for the sake of demonstration.
You shouldn't ever do this in real realtime code (because print statements usually allocate!)

[Here is a link](https://play.rust-lang.org/?gist=6e37aa0a7f8d06f8b31b9822c8bbb79c&version=stable&backtrace=0) to this code in the Rust playground.
It might timeout if you try running it. If you see any messages about timeout, don't worry, just try running the code again.

Here is an example output:
```
[realtime] thread started
[ui] thread started
[ui] sending new samples. Second sample: 0
[realtime] received new samples. Second sample: 0
[ui] sending new samples. Second sample: 0.009801715
[realtime] received new samples. Second sample: 0.009801715
[ui] sending new samples. Second sample: 0.01960343
[realtime] received new samples. Second sample: 0.01960343
[ui] sending new samples. Second sample: 0.029405143
[realtime] received new samples. Second sample: 0.029405143
[ui] sending new samples. Second sample: 0.03920686
[realtime] received new samples. Second sample: 0.03920686
[realtime] thread shutting down
[ui] thread shutting down
```

## Problems?
The last example *seems* to do the right thing, let's take a look at what the realtime callback does when it receives a new set of samples.

```rust
// ...
Message::NewSamples(samples) => {
    self.current_samples = Some(samples)
},
// ...
```

What happens to the old array of samples?
Rust will insert a call to `drop` here, because the old value has just gone out of scope.
Something like this (in pseudo-Rust) sort of shows what is going on.

```rust
// ...
Message::NewSamples(samples) => {
    let tmp = Some(samples);
    mem::swap(self.current_samples, tmp);
    drop(tmp);
},
// ...
```

When an `Arc` gets `drop`ped, what happens?
Let's refer to the docs for `drop`.

> This will decrement the strong reference count. If the strong reference count becomes zero and the only other references are Weak<T> ones, drops the inner value.

In this case, the inner value is some heap allocated memory, so calling drop will deallocate that memory (since no one else is holding any references).
This is a problem!
We can't let our realtime callback perform memory allocation.

# Build the GC

We now need to build the GC thread, to clean up after us, outside of the realtime thread.
Sneak peak, once the GC is implemented, all we have to change is `UIThread::run`, in a very small way:

```rust
/// All of the UI thread code
fn run(&mut self) {
    let mut gc = GC::new(); // + NEW LINE

    // create 5 "ui events"
    for i in 0..5 {
        let volume = i as f32 / 5.0;
        let samples = Arc::new(self.compute_samples(volume));
        self.collector.track(samples.clone()); // + NEW LINE

        // send the samples to the other thread
        println!("[ui] sending new samples. Second sample: {}", samples[1]);
        self.outgoing.send(Message::NewSamples(samples)).unwrap();
    }

    // tell the other thread to shutdown
    self.outgoing.send(Message::Shutdown).unwrap();
}
```

With that in mind, lets sketch out the interface for the Garbage Collector.

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T> {
    // ...
}

impl<T> GC<T> {
    /// Construct a new garbage collector and start the collection thread
    fn new() -> Self {
        // ...
    }

    /// Instruct the garbage collector to monitor this Arc<T>
    /// When no references remain, the collector will `drop` the value
    fn track(&mut self, t: Arc<T>) {
        // ...
    }
}

```

First think about the `track` method.
All this method needs to do is move it's argument into some list (vector) of pointers.
We will keep this vector in the GC thread struct so that each of the references will live until the GC thread is shut down or until the GC drops them.

```rust
struct GC<T> {
    pool: Vec<Arc<T>>,
}

impl<T> GC<T> {
    // ...

    pub fn track(&mut self, t: Arc<T>) {
        self.pool.push(t);
    }
}
```

Now lets think about the garbage collection logic.
Since we have a `Vec<Arc<T>>`, we will want to iterate over it, removing any elements which meet (or fail) a condition.
We can use `Vec::retain` to do this.
Something like the following might work:

```rust
pool.retain(|e| {
    if /* has more than one reference */ {
        return true
    } else {
        return false
    }
})
```

Looking at the [`Arc` docs](https://doc.rust-lang.org/std/sync/struct.Arc.html), there are a few ways we can figure out if the `Arc` has only one remaining reference:
* Attempt to consume the `Arc` with `Arc::try_unwrap`, if this fails, we know that it has more than one reference. Unforunately, this method requires moving the `Arc` out of the vector, which is not ideal if we want to use `Vec::retain`.
* `Arc::strong_count` - this is currently marked as unstable. Looks like what we might want to use though.
* `Arc::get_mut` could possibly be used the same way we would use `Arc::try_unwrap`, without moving the `Arc` containing in the vector unless we want to remove it.

We don't have lots of options, so I'm going to go ahead and use `Arc::strong_count`.
This is (for now) the most natural way to solve the problem:

```rust
pool.retain(|e: Arc<_>| {
    if Arc::strong_count(&e) {
        return true
    } else {
        return false
    }
})
```

Let's move on to `new`.
The `new` method needs to start new thread which will run the `pool.retain` thing every once and a while.
We also need to hold on to a thread handle so that we can eventually join the thread.
The join handle is wrapped in an `Option`, we will see why quite a bit later.

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T> {
    pool: Vec<Arc<T>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl<T> GC<T> {
    // private. cleans up any dead pointers in a pool
    fn cleanup(pool: &mut Vec<Arc<T>>) {
        pool.retain(|e: &Arc<_>| {
            if Arc::strong_count(&e) > 1 {
                return true
            } else {
                return false
            }
        });
    }

    pub fn new() -> Self {
        let pool = Vec::new();

        // create a closure which will become a new thread
        let gc = || {
            loop {
                GC::cleanup(&mut pool);

                // wait for 100 milliseconds, then scan again
                let sleep = std::time::Duration::from_millis(100);
                thread::sleep(sleep);
            }
        };

        // spawns a new thread and returns a handle to the thread
        let gc_thread = thread::spawn(gc);

        GC {
            pool:   pool,
            thread: Some(gc_thread),
        }
    }

    pub fn track(&mut self, t: Arc<T>) {
        self.pool.push(t);
    }
}

fn main() {
    let (tx, rx) = mpsc::sync_channel(0);
    let rt = RealtimeThread::new(rx);
    let ui = UIThread::new(tx);
    run_threads(rt, ui);
}
```

We written a bunch of new code, better make sure it compiles ([Rust playground](https://play.rust-lang.org/?gist=0740c7896b0dd8c37e1d57aa9e53ca0b&version=stable&backtrace=0)):

```rust
error[E0277]: the trait bound `T: std::marker::Send` is not satisfied
   --> <anon>:154:25
    |
154 |         let gc_thread = thread::spawn(gc);
    |                         ^^^^^^^^^^^^^ the trait `std::marker::Send` is not implemented for `T`
    |
    = help: consider adding a `where T: std::marker::Send` bound
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::sync::Arc<T>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::ptr::Unique<std::sync::Arc<T>>`
    = note: required because it appears within the type `alloc::raw_vec::RawVec<std::sync::Arc<T>>`
    = note: required because it appears within the type `std::vec::Vec<std::sync::Arc<T>>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `&mut std::vec::Vec<std::sync::Arc<T>>`
    = note: required because it appears within the type `[closure@<anon>:143:18: 151:10 pool:&mut std::vec::Vec<std::sync::Arc<T>>]`
    = note: required by `std::thread::spawn`

error[E0277]: the trait bound `T: std::marker::Sync` is not satisfied
   --> <anon>:154:25
    |
154 |         let gc_thread = thread::spawn(gc);
    |                         ^^^^^^^^^^^^^ the trait `std::marker::Sync` is not implemented for `T`
    |
    = help: consider adding a `where T: std::marker::Sync` bound
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::sync::Arc<T>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::ptr::Unique<std::sync::Arc<T>>`
    = note: required because it appears within the type `alloc::raw_vec::RawVec<std::sync::Arc<T>>`
    = note: required because it appears within the type `std::vec::Vec<std::sync::Arc<T>>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `&mut std::vec::Vec<std::sync::Arc<T>>`
    = note: required because it appears within the type `[closure@<anon>:143:18: 151:10 pool:&mut std::vec::Vec<std::sync::Arc<T>>]`
    = note: required by `std::thread::spawn`

error: aborting due to 2 previous errors
```

Oops, this isn't good.
This error makes it feel sort of like Rust hates us, but the compiler is actually doing us a massive favor.

In Rust, there are a few thread safety "marker traits" called `Send` and `Sync`.
The compiler is telling us that our generic type `T` doesn't implement either of them.

Put very loosely, if something implements `Send`, it is safe to send it between threads.
`Sync` is considerably more subtle and quite difficult to wrap your head around, but we can sort of say that, if something implements `Sync`, we can *access* the same instance of it from multiple threads.
For more info, you can read [this blog post](http://huonw.github.io/blog/2015/02/some-notes-on-send-and-sync/), but you shouldn't need any more than what I've given to get through the rest of my post.

So anyway, Rust is telling us that we have a thread safety problem, but we haven't guaranteed that we can safely copy and access values of our type `T` between the garbage collector thread and any other threads.

I know that `T` must be `Send`, because it has to be sent between threads, so let's go ahead and add that restriction:

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T: Send> {
    pool: Vec<Arc<T>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl<T: Send> GC<T> {
// ....
```

[Rust playground link](https://play.rust-lang.org/?gist=4f718d3a1795409d67894a8f4f86f010&version=stable&backtrace=0)

Hoorary, the `Send` error is gone!
Unfortunately, we still have the issue with `Sync`.
Let's look more closely at the error we are getting:

```
error[E0277]: the trait bound `T: std::marker::Sync` is not satisfied
   --> <anon>:154:25
    |
154 |         let gc_thread = thread::spawn(gc);
    |                         ^^^^^^^^^^^^^ the trait `std::marker::Sync` is not implemented for `T`
    |
    = help: consider adding a `where T: std::marker::Sync` bound
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::sync::Arc<T>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `std::ptr::Unique<std::sync::Arc<T>>`
    = note: required because it appears within the type `alloc::raw_vec::RawVec<std::sync::Arc<T>>`
    = note: required because it appears within the type `std::vec::Vec<std::sync::Arc<T>>`
    = note: required because of the requirements on the impl of `std::marker::Send` for `&mut std::vec::Vec<std::sync::Arc<T>>`
    = note: required because it appears within the type `[closure@<anon>:143:18: 151:10 pool:&mut std::vec::Vec<std::sync::Arc<T>>]`
    = note: required by `std::thread::spawn`

error: aborting due to previous error
```

This error is really confusing, and my solution for it is not going to be much better, but stick with me.

The origin of this error is the `Arc<T>`.
If we want an `Arc<T>` to implement `Send`, the `T` contained in it must implement BOTH `Send` and `Sync`.
It makes sense that `T` would need to implement `Send`, but why does `T` need to be `Sync`?
Basically, this is because the data the `Arc<T>` is holding will be shared by anyone who can access the `Arc<T>`.
An `Arc` can be `clone`ed at any time, so, if we are allowed to pass it to other threads, it must also be safe for multiple threads to access the underlying data at the same time.

We could add the `Sync` constraint to our type `T` to resolve this problem, but does this really make any sense?
Nowhere in our application will a message be accessible by more than one thread at a time.

When the UI thread creates a new message, it immediately surrenders all access to the underlying data, by moving the value into the channel.
Once the realtime thread has the data, it will be the only thread that actually accesses the data until the data needs to be freed.
The GC also is holding a reference to data, but it will never actually touch the data in any way, until it frees it.
When the GC thread frees the memory holding the data, we know that there will be no other references to the memory in the program.

I might be wrong about this (please let me know if I am), but I think that we don't actually *need* the type `T` to be `Sync`.
The compiler will never let us get away with this (because it doesn't know all of these properties) but we can let it know that it should trust us, with a new struct:

```rust
struct TrustMe<T> {
    pub inner: T
}

unsafe impl<T> Send for TrustMe<T> {}
```

This will tell the compiler "yes, this thing is `Send`", even when it actually isn't, so the implementation of the trait `Send` is unsafe.

Now, we can create a `Send`able `TrustMe<Arc<T>>`, and the compiler will trust us when we share these `Arc<T>`s between threads.

Now, lets add this to our GC:

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T: Send> {
    pool: Vec<TrustMe<Arc<T>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl<T: Send> GC<T> {
    // private. cleans up any dead pointers in a pool
    fn cleanup(pool: &mut Vec<TrustMe<Arc<T>>>) {
        pool.retain(|e: &TrustMe<Arc<_>>| {
            if Arc::strong_count(&e.inner) > 1 {
                return true
            } else {
                return false
            }
        });
    }

    pub fn new() -> Self {
        let mut pool = Vec::new();

        // create a closure which will become a new thread
        let gc = || {
            loop {
                GC::cleanup(&mut pool);

                // wait for 100 milliseconds, then scan again
                let sleep = std::time::Duration::from_millis(100);
                thread::sleep(sleep);
            }
        };

        // spawns a new thread and returns a handle to the thread
        let gc_thread = thread::spawn(gc);

        GC {
            pool:   pool,
            thread: Some(gc_thread),
        }
    }

    pub fn track(&mut self, t: Arc<T>) {
        let t = TrustMe { inner: t };
        self.pool.push(t);
    }
}
```

[Rust Playground Link](https://play.rust-lang.org/?gist=b23d5e7a7541eda3096daac685d309bf&version=stable&backtrace=0)

When we try to compile this, we get YET ANOTHER compiler error.
This time, the compiler is whining at us with "the parameter type `T` may not live long enough".
This error message is frustrating, but, we are using Rust because we want to be very careful with memory safety, so lets try to keep going.

The new thread that we have created could run until the termination of the program, so any data which the thread might be holding onto also must be able to live until the termination of the program.

The compiler is telling us that we need to add a "lifetime specifier" to our type `T`.
In this case, it is telling us that the lifetime of any `T` which is managed by the GC must be `'static`.
The `'static` lifetime indicates that values of type `T + 'static` *might* live for the entire duration of the program.

This might seem excessive, but, it is not possible for the compiler to determine when in the program our thread will terminate (if it could we would have solved the halting problem), so the maximum lifetime MUST potentially be the entire duration of the program.
Note that, this doesn't mean that all the values stored in the GC will necessarily live for the entire lifetime of the program (if they did, we wouldn't be cleaning up garbage).
This condition just means that they might live that long.

Anyway, we can now add the `+ 'static` specifier the compiler has asked us to add, and try to compile this one more time.

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T: Send + 'static> {

// ...

impl<T: Send + 'static> GC<T> {

// ...
```

GUESS WHAT IT DIDN'T WORK.

```
error[E0373]: closure may outlive the current function, but it borrows `pool`, which is owned by the current function
   --> <anon>:149:18
    |
149 |         let gc = || {
    |                  ^^ may outlive borrowed value `pool`
150 |             loop {
151 |                 GC::cleanup(&mut pool);
    |                                  ---- `pool` is borrowed here
    |
help: to force the closure to take ownership of `pool` (and any other referenced variables), use the `move` keyword, as shown:
    |         let gc = move || {

error: aborting due to previous error
```

Once again, this is a good thing, I promise!
Now, the compiler is trying to tell us that the vector named `pool` is being accessed from two different places.
The compiler wants us to have the new thread take ownership of the vector, but this highlights an interesting problem.
We need to allow both the GC thread, and any other non-realtime thread, to access the vector, at the same time.

The compiler has prevented us from accessing the same data from multiple threads.

To solve this, we can just wrap the vector in a `Mutex` **and** an `Arc`.
The `Arc` allows us to create one instance of the vector on the heap, and the `Mutex` makes sure that only one thread can access the heap allocated vector at any given time.

Here are most of the changes:

```rust
// introduce some news type aliases to make life a little bit easier
type TrustedArc<T> = TrustMe<Arc<T>>;
type ArcPool<T> = Vec<TrustedArc<T>>;

/// A garbage collector for Arc<T> pointers
struct GC<T: Send + 'static> {
    pool: Mutex<Arc<ArcPool<T>>>,
    thread: Option<thread::JoinHandle<()>>,
}

// ...

impl<T: Send + 'static> GC<T> {
    // ...
    pub fn new() -> Self {
        let pool = Arc::new(Mutex::new(Vec::new()));

        // create a copy of the pool. The GC thread will own this clone
        // and the reference count will be incremented by one
        let thread_arc_copy = pool.clone();

        // create a closure which will become a new thread
        let gc = move || {
            loop {
                // lock the mutex, then let go of it.
                // If we hold the mutex, the UI thread will be blocked every time it asks the
                // collector to track something.
                {
                    let mut pool = thread_arc_copy.lock().unwrap();
                    GC::cleanup(&mut pool);
                }

                // wait for a bit, then scan again
                let sleep = std::time::Duration::from_millis(5);
                thread::sleep(sleep);

            }
        };

        // ....
    }

    pub fn track(&mut self, t: Arc<T>) {
        let t = TrustMe { inner: t };
        let mut pool = self.pool.lock().unwrap();
        pool.push(t);
    }
}
```

We can finally compile this!
Here's a link to the [Rust playground](https://play.rust-lang.org/?gist=7f41622e104d07f9b106495c2a5373a7&version=nightly&backtrace=0).
Note that you will need to make sure you compile with the "Nightly" channel.

There are only a few things left to do.

## Start and Stop the GC
The GC thread that we have created will never terminate.

Ideally, when the GC goes out of scope, it will shut down the GC thread and clean up any tracked memory (if it can).
Any `Arc`s which can't be freed when the GC is shut down will not be freed, but (this is important) the reference count will drop by one.
Now, if one of the previously tracked `Arc`s goes out of scope, it will be freed on whatever thread drops it (this could be the realtime thread!)

So, as long as the realtime thread keeps running, we must keep the GC thread running.

First, edit main:

```rust
fn main() {
    // start the collector
    let collector = GC::new();

    // create the channels
    let (tx, rx) = mpsc::sync_channel(0);

    // set up both of the threads
    let rt = RealtimeThread::new(rx);
    let ui = UIThread::new(tx);

    // start the threads
    run_threads(rt, ui);

    // GC thread will be shutdown here, where the GC goes out of scope
}
```

Then, edit the `UIThread` struct appropriately.

```rust

struct UIThread {
    outgoing: mpsc::SyncSender<Message>,
    collector: GC<Samples>
}

impl UIThread {
    fn new(outgoing: mpsc::SyncSender<Message>, collector: GC<Samples>) -> Self {
        UIThread { outgoing: outgoing, collector: collector }
    }

    // ...
}
```

Next, update the `UIThread::run` method:

```rust
    /// All of the UI thread code
    fn run(&mut self) {
        // create 5 "ui events"
        for i in 0..5 {
            let volume = i as f32 / 5.0;
            let samples = Arc::new(self.compute_samples(volume));

            // tell the GC thread to track our list of samples
            self.collector.track(samples.clone());

            // send the samples to the other thread
            println!("[ui] sending new samples. Second sample: {}", samples[1]);
            self.outgoing.send(Message::NewSamples(samples)).unwrap();
        }

        // tell the other thread to shutdown
        self.outgoing.send(Message::Shutdown).unwrap();
    }
```

## Drop the GC

Rust will make sure that `Drop` is called when the struct goes out of scope.
This gives us a change to shut down the GC thread.
We also set up a shared atomic boolean to indicate when the GC thread should shut down.

Here is most of that:

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T: Send + 'static> {
    pool: Arc<Mutex<ArcPool<T>>>,
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

// initialize the running flag to false in GC::new

// ....

impl<T: Send + 'static> Drop for GC<T> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        match self.thread.take() {
            Some(t) => t.join().unwrap(),
            None    => ()
        };
    }
}
```

And, here's the [Rust playground link](https://play.rust-lang.org/?gist=c33dec5b9aad44864035de4c81c1f492&version=nightly&backtrace=0).
You may have some trouble getting this to run (timeouts occur), but I promise it works sometimes.

Example output:

```
[realtime] thread started
[ui] thread started
[ui] sending new samples. Second sample: 0
[ui] sending new samples. Second sample: 0.01960343
[realtime] received new samples. Second sample: 0
[ui] sending new samples. Second sample: 0.03920686
[realtime] received new samples. Second sample: 0.01960343
[realtime] received new samples. Second sample: 0.03920686
[ui] thread shutting down
[realtime] thread shutting down
```

## Proof
Let's add some logging so we can see when things are getting freed:

```rust
// private. cleans up any dead pointers in a pool
fn cleanup(pool: &mut Vec<TrustMe<Arc<T>>>) {
    pool.retain(|e: &TrustMe<Arc<_>>| {
        if Arc::strong_count(&e.inner) > 1 {
            return true
        } else {
            println!("[gc] dropping a value!");
            return false
        }
    });
}
```

The completed code lives at [this Rust playground link](https://play.rust-lang.org/?gist=7c48a9e595463cb4b8a2c155feb50234&version=nightly&backtrace=0).

Example Output:
```
[realtime] thread started
[ui] thread started
[ui] sending new samples. Second sample: 0
[realtime] received new samples. Second sample: 0
[ui] sending new samples. Second sample: 0.01960343
[realtime] received new samples. Second sample: 0.01960343
[gc] dropping a value!
[ui] sending new samples. Second sample: 0.03920686
[realtime] received new samples. Second sample: 0.03920686
[gc] dropping a value!
[ui] sending new samples. Second sample: 0.058810286
[realtime] received new samples. Second sample: 0.058810286
[gc] dropping a value!
[ui] sending new samples. Second sample: 0.07841372
[realtime] received new samples. Second sample: 0.07841372
[gc] dropping a value!
[ui] thread shutting down
[realtime] thread shutting down
```

# Conclusion
We did it!

For me, this post exemplifies the reasons I am so excited about Rust.
The realtime audio world places us into a world where many programming languages are simply not usable.
Languages with runtimes that may behave unpredictably cannot meet the extremely strict requirements we must meet for correct realtime operation.
Rust allows us to meet all of those requirements and gives us some nice abstractions.

On top of that, the Rust compiler meticulously checks for thread safety violations and memory safety violations.
While writing this post, some of the issues the compiler threw at me (`'static`, for example), are issues I never considered.
The compiler caught me and told me "no," so I had to think about what was actually going on.

These checks are absolutely irritating, and sometimes we might want to work around them (like we did with `TrustMe`).
I'm glad to be exposed to potential issues, even if I have to work around the compiler sometimes (so far).

If you made it this far, thank you for reading.
I hope you've learned something interesting (maybe even useful).
