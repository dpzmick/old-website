---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on a synthesizer (the kind that makes sounds) in Rust.
This post is the second post in a series of posts about this project.
See my [blog series](/series/) page for links to other posts.

If you don't know anything at all about realtime audio programming, you might want to read the first post in this series, [Audio Programming 101](/2016/12/17/audio-basics/), to get a little bit of background.
Basically, there's a realtime thread that can never be blocked in any way.
The realtime thread is responsible for generating all of the audio which an application will produce.
If the realtime thread ever fails to generate the audio it needs to generate, bad things happen.
That means locks, I/O, and allocation are all off limits in the realtime thread.
Sending messages from non-realtime threads to the realtime thread is trickier than it might be in a "normal" application because we can't do these things.
There are many, many techniques which can be used to work around this trickiness.
This post is a discussion of one such method (presented in [this cppcon talk](https://www.youtube.com/watch?v=boPEO2auJj4)) implemented in [Rust](https://www.rust-lang.org/en-US/).

# Messaging between threads
Suppose we are developing a synthesizer which produces sounds when keys are pressed on a [MIDI keyboard](https://en.wikipedia.org/wiki/MIDI_controller#Keyboards).
The audio library we are using calls a function we provide once ever 6 or so milliseconds to request a list of samples from us.
The library calls our function with 2 arguments: 1) How many samples it wants 2) what key presses we need to handle.
The callback function uses a precomputed list of samples to generate sounds every time it is called.
To modify the properties of the sounds that are produced, the user edits the synthesizer settings with a user interface.

It would be painful (and incorrect) to attempt to handle UI events in the realtime thread, so we will run a UI thread to handle the UI events.
Whenever the UI thread gets an event to handle, it needs to compute a new sample list, then send the list to the realtime thread.

If we were allowed to lock, we could just stick a `mutex` around a global list of samples and call it a day, but we can't do that.

Instead of locking, lets use a queue to send some sort of message between threads.
The queue that we choose needs to have a few properties:

* Must be a [lock free queue](https://pdfs.semanticscholar.org/a909/1ef790788c5d252cad94dd6862adf457e073.pdf)
* Must be able to preallocate all of its nodes (cant't allocate and free a node object on a push or pull)

I would like place this message on the heap so it doesn't need to be copied each time a new thread takes ownership of it.
If the message lives on the heap, we must ensure that the message is allocated and freed outside of the realtime thread.
These messages can be kept simple.
All we need is a tag (what kind of message this is), and some block of memory to contain samples.

## Reference counted garbage collection
Because these messages are going to live on the heap, they will need to be allocated and freed.
Since the messages are created by the UI thread, it is fine for us to allocate space for the message, populate it, then ship a pointer over to the realtime thread.
When the realtime thread takes ownership of the message, it will need to hold onto the data for some undefined period of time.
When the realtime thread is done with the message, it cannot free it, because we can't call memory allocation functions in the realtime thread.

To solve this, we will run one more thread to clean up messages which are no longer being used by the realtime thread.
Whenever the UI thread creates a message, it will wrap it in a reference-counted pointer.
It then will let the collector thread know it should start tracking the reference-counted pointer.
The message is then sent over the queue from the UI thread to the realtime thread.
When the realtime thread receives the message, it will keep a reference to the message until it is done with it.

The collector thread stores a list of pointers which it manages, so every pointer which is managed by the collector will always have at least one reference.
Every 100 milliseconds or so, the collector will scan its pointer list, removing anything which has a reference count of 1.
When the elements are removed from the list, their memory is freed.

[Here](/img/sound/gc_queue.pdf) is a slideshow/animation demonstrating this process.

## Tradeoffs
This approach is useful when the message producer blah blah blah but not when it blah blee bloo.

# Let's get building
Now lets make one in Rust.
For the sake of these examples, let's assume that the built in Rust [mpsc channel](https://doc.rust-lang.org/std/sync/mpsc/index.html) is an appropriate lock free queue.
It will be pretty easy to swap this with something different later, and, if we use the standard library, all of the examples will easily run in the rust playground.
We are also going to fake a bunch of the details of the audio library.

## Fake audio library
Rust playground link: [https://is.gd/Qe1YjZ](https://is.gd/Qe1YjZ)

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
Now that we have an "audio library," lets try to make some messages and pass them between threads.
I'm going to jump right into the "garbage collector" solution here.
Other solutions will be discussed elsewhere.

The `RealtimeThread` struct will need to hold on to a list of samples which it will use to populate the `output` samples every time the callback is called.
We want these samples to be heap allocated and reference counted, so we wrap them in an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html).
Finally, we want to leave the samples uninitialized until the UI thread sends us some, so we wrap the `Arc<Samples>` in an [`Option`](https://doc.rust-lang.org/std/option/enum.Option.html).

  ```rust
  struct RealtimeThread {
      current_samples: Option<Arc<Samples>>,
  }
  ```

  ```python
  def test():

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

Notice that it returns the list of samples by value.
Have no fear, [return value optimization](https://en.wikipedia.org/wiki/Return_value_optimization) is here!
I don't actually know if Rust will perform this optimization (TODO VERIFY THIS), but it doesn't really matter for this example.
If we do end up copying the samples, it doesn't matter that much.
Even if the copy is slow, we will be performing the copy on the UI thread, where we can afford to be a bit slower.

The UI thread will generate some fake events, and compute samples for these events:

```rust
/// All of the UI thread code
fn run(&mut self) {
    // create 10 "ui events"
    for i in 0..10 {
        let volume = i as f32 / 10.0;
        let samples = Arc::new(self.compute_samples(volume));

        // send the samples to the other thread
    }

    // tell the other thread to shutdown
}
```

Now that we've done all of that, we need to send the samples between threads.
We need a message type.
Messages from the UI thread will be either a new list of samples or a request to shutdown.
Since we are storing the samples as an `Arc`, we will send them to the realtime thread as an `Arc`.

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
This means that the channel will not buffer any messages.
From [the docs](https://doc.rust-lang.org/std/sync/mpsc/fn.sync_channel.html):

> Note that a buffer size of 0 is valid, in which case this [channel] becomes "rendezvous channel" where each send will not return until a recv is paired with it.

To create this queue, first we need to add some code to `main` to create the queues:

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
In both cases, if the send fails, something has gone horribly wrong, so its fine to `unwrap` the result of these sends.

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

            Message::Shutdown => return CallbackStatus::Shutdown
        },

        // if we failed to receive anything, just keep sending samples
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
You shouldn't ever do this in real realtime code.
Print statements are not usually implemented in a realtime safe manner.

This code is to long to past a Rust playground link to, so [here](/code/sound/arc1.rs) is the full source.
You can copy/paste the code in to run it.

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

# Collecting the garbage
The last example seems to be working, but it has one fatal flaw.
Let's take a look at what the realtime callback does when it receives a new set of samples.

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

We aren't currently holding on to any other references to this block of memory, so this means that the samples will be deallocated.
This is a problem! We can't let our realtime callback perform memory allocation.
One (of many) ways to fix this would be to deallocate the list of samples from another thread once no one has a reference to them anymore.
Luckily, an `Arc` is reference counted, so we know that we can access this count.
Essentially, we want to create a lightweight garbage collector that will clean up our reference counted, heap allocated samples when we are done with them.
Sneak peak, once the GC is implemented, all we have to change is `UIThread::run`, in a very small way:

```rust
    /// All of the UI thread code
    fn run(&mut self) {
        let mut gc = GC::new(); // + NEW LINE

        // create 10 "ui events"
        for i in 0..5 {
            let volume = i as f32 / 10.0;
            let samples = Arc::new(self.compute_samples(volume));
            gc.track(samples.clone()); // + NEW LINE

            // send the samples to the other thread
            println!("[ui] sending new samples. Second sample: {}", samples[1]);
            self.outgoing.send(Message::NewSamples(samples)).unwrap();
        }

        // tell the other thread to shutdown
        self.outgoing.send(Message::Shutdown).unwrap();
    }
```

From the last example, we can roughly define the interface we want our garbage collector to have:
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

Lets start filling in some of these methods.
First lets think about the `track` method.
Let's just hold onto a vector of all of the pointers we are currently tracking.
This serves two purposes:
1. Gives us a convinient way to iterate over the data we are tracking.
2. Make sure there is always at least 1 reference to the data we are tracking (our own)

We care about the second because we never want these `Arc<T>`s to be dropped outside of the collector.

Lets go ahead and give this a try:
```rust
struct GC<T: Send + 'static> {
    pool: Vec<Arc<T>>,
}

impl<T> GC<T> {
    // ...

    pub fn track(&mut self, t: Arc<T>) {
        self.pool.push(t);
    }
}
```

Looks okay to me, now lets think about the logic we will need to implement to do garbage collection.
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




Looks okay to me, lets move on to `new`.
`new` needs to create a new thread which runs the `pool.retain` thing we just wrote.
Additionally, if we are going to start a new thread, we will also need to hold on to a handle to join that thread when we shut down.

```rust
/// A garbage collector for Arc<T> pointers
struct GC<T> {
    pool: Vec<Arc<T>>,
    thread: thread::JoinHandle<()>,
}

impl<T: Send + 'static> GC<T> {
    pub fn new() -> Self {
        let pool = Vec::new();

        let gc = || {
            loop {
                pool.retain(|e: &Arc<_>| {
                    if Arc::strong_count(&e) > 1 {
                        return true
                    } else {
                        return false
                    }
                });

                let sleep = std::time::Duration::from_millis(100);
                thread::sleep(sleep);
            }
        };

        let gc_thread = thread::spawn(gc);

        GC {
            pool:   pool,
            thread: gc_thread
        }
    }

    pub fn track(&mut self, t: Arc<T>) {
        self.pool.push(t);
    }
}
```

We written a bunch of new code, better make sure it compiles:

```rust
$ rustc test.rs
error[E0277]: the trait bound `T: std::marker::Sync` is not satisfied
  --> test.rs:64:25
   |
64 |         let gc_thread = thread::spawn(gc);
   |                         ^^^^^^^^^^^^^ trait `T: std::marker::Sync` not satisfied
   |
   = help: consider adding a `where T: std::marker::Sync` bound
   = note: required because of the requirements on the impl of `std::marker::Send` for `std::sync::Arc<T>`
   = note: required because of the requirements on the impl of `std::marker::Send` for `std::ptr::Unique<std::sync::Arc<T>>`
   = note: required because it appears within the type `alloc::raw_vec::RawVec<std::sync::Arc<T>>`
   = note: required because it appears within the type `std::vec::Vec<std::sync::Arc<T>>`
   = note: required because of the requirements on the impl of `std::marker::Send` for `&mut std::vec::Vec<std::sync::Arc<T>>`
   = note: required because it appears within the type `[closure@test.rs:49:18: 62:10 pool:&mut std::vec::Vec<std::sync::Arc<T>>]`
   = note: required by `std::thread::spawn`

error: aborting due to previous error
```

Oops, this isn't good.

Let's try to break this down (emphasis mine):

<!--

## `memcpy` queue
My first solution was the simplest thing I could come up with.
I set up a fixed sized queue between the two threads.
The queue sent complete lists of samples from the UI thread to the realtime thread.
Here's (roughly) how this works:

* The UI thread notices it needs to handle some UI event
* The UI thread recomputes the list of samples and stores them on it's stack
* The UI thread sends the samples to the other thread over the queue (`memcpy` the samples into the queue's buffer)
* Every time the realtime callback is called, it checks the queue for new messages.
* If there are new messages, the samples are `memcpy`ed out of the queue's buffer, into the callback's private buffer (which also lives on the stack).

For a crappy slide show demonstrating this process, [click here](/img/sound/memcpy_queue.pdf).
To get this right, we must be careful with the queue that we use.

First, the queue must not use locks to send messages back and forth.
This we can deal with pretty easily; there are many good lock free queue and ringbuffer implementations.

Second, we must be pretty careful about allocation.
Many queues will create a new "queue node" to hold the data placed on the queue.
When the data is pulled from the queue, the node is deallocated.
We cannot deallocate any queue nodes in the realtime thread.
We also probably shouldn't leak the nodes either, so we need to be careful about allocation of queue nodes.

If we are sure to preallocate everything that the queue will need, and we use a good lock free queue implementation, we get around both of these issues.

Finally, note that it I am totally fine letting the UI thread wait for for space in the queue if the realtime thread is not consuming events fast enough.
I'm assuming that my UI thread is not going to be generating events significantly faster than the realtime thread can consume them.
If it does, there are ways to work around this on the UI thread which I will not get into now (maybe in a future post).

It looks like we can probably pull this off.
`memcpy` is pretty fast, so we can probably afford to do a large `memcpy` in the realtime thread every once and a while, but it's not a very good idea to do something slow at effectively random times in the realtime thread.
It also just feels wrong to make so many copies.
I'm sure we can do better.

## Pointer queue
All problems in computer science can be solved by adding a layer of indirection (or so they say).
Let's try to get rid of all of these copies with pointers!
We will heap allocate some samples samples on the UI thread, populate them, then pass a pointer to those samples to the realtime thread.
Again, we need to use a carefully constructed, bounded, lock-free queue.

When there is a message available for the realtime thread, all it has to do is swap its `current` pointer with the new pointer that came over the queue.

But wait!
What will we do with the previous list of samples?
We can't free this memory in the realtime thread, and we definitely don't want to leak it, so we need to send it to some other thread to be freed (or reused).
Let's just send the memory back to the UI thread, then let the UI thread deal with it (perhaps it can even reuse the memory).

This seems plausible, but there are some complications we must work around:
1. We cannot use an unbounded queue to send samples from the realtime thread to the UI thread.
    * As far as I know, there are no unbounded queues which do not perform allocation when sending messages.
2. We cannot use a blocking, bounded queue to send the memory back to the UI thread.

TODO this might actually be doable

To understand the first of these, consider this scenario.
If the queue is full, we will have to wait until there is room on the queue to push the new pointers.
Remember that the UI thread is not running in realtime.
It might not have been run by the scheduler recently, so it may not have had a chance to drain the queue the next time the realtime thread

* We cannot cache the pointers in the realtime thread when the queue is full
    * The cache of pointers would need to be unbounded (the required size is non-deterministic).
        * In order to create an unbounded list of something, we need to allocate.

### Double buffering
If we get rid of the queue and replace it with two pointers, we swap between the two states with some atomic operations.
[JackAtomicState.h](https://github.com/jackaudio/jack2/blob/364159f8212393442670b9c3b68b75aa39d98975/common/JackAtomicState.h) is an example of such a thing.
Unfortunately, we must keep in mind that the UI thread is not realtime.
If we want to avoid leaks, we must make sure that we never overwrite something before it is freed.

I'm sure there are some ways to work around all the double buffering/pointer queue issues, but frankly, I don't want to, my gut and a few hours of thought have declared any solutions I've come up with too complicated for comfort.

# A "Garbage Collector"
At this point, I considered shipping reference counted objects around to manage these issues (and use the reference count to figure out when to free them).
I wasn't totally confident that this was a good idea, until I watched the [cppcon video](https://www.youtube.com/watch?v=boPEO2auJj4), in which the speaker builds a simple "garbage collector" for `std::shared_ptr` and says "yes this is a good idea."

Here begins the discussion about how I've done the same thing in rust.

TODO touchy bit about page faults

# rust stuff

{% highlight rust %}
#![feature(arc_counts)]

use std::sync::mpsc::SyncSender;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time;

struct TrustMe<T> {
    pub data: T
}

//unsafe impl<T> Sync for TrustMe<T> {}
unsafe impl<T> Send for TrustMe<T> {}

/// Doesn't do anything with the pointer until it has no references other than itself
struct GC<T: Send + 'static> {
    pool: Arc<Mutex<Vec<TrustMe<Arc<T>>>>>,
    thread: Option<thread::JoinHandle<()>>,
    notify: SyncSender<bool>,
}

impl<T: Send + 'static> GC<T> {
    pub fn new() -> Self {
        let pool = Arc::new(Mutex::new(Vec::new()));

        let (rx, tx) = mpsc::sync_channel(0);

        let tpool = pool.clone();
        let gc = move || {
            loop {
                match tx.try_recv() {
                    Ok(_)  => break,
                    Err(_) => ()
                };

                let mut pool = tpool.lock().unwrap();
                pool.retain(|e: &TrustMe<Arc<_>>| {
                    if Arc::strong_count(&e.data) > 1 {
                        return true
                    } else {
                        println!("doing a drop");
                        return false
                    }
                });

                let ten_millis = time::Duration::from_millis(10);
                thread::sleep(ten_millis);
            }
        };

        let gc_thread = thread::spawn(gc);

        GC {
            pool: pool,
            thread: Some(gc_thread),
            notify: rx,
        }
    }

    pub fn track(&mut self, t: Arc<T>) {
        let mut p = self.pool.lock().unwrap();
        let trust = TrustMe { data: t };
        p.push(trust);
    }
}

impl<T: Send + 'static> Drop for GC<T> {
    fn drop(&mut self) {
        println!("collector going down!");
        self.notify.send(true).unwrap();

        let t = self.thread.take();
        match t {
            Some(t) => t.join().unwrap(),
            None    => ()
        }
    }
}

struct LoudDrop { }
impl LoudDrop {
    pub fn new() -> Self { LoudDrop {} }
}

impl Drop for LoudDrop {
    fn drop(&mut self) {
        println!("being dropped")
    }
}

fn main() {
    let mem = Arc::new(LoudDrop::new());
    {
        let mut collector = GC::<LoudDrop>::new();
        collector.track(mem.clone());

        {
            let mem = Arc::new(LoudDrop::new());
            collector.track(mem.clone());

            let ten_millis = time::Duration::from_millis(1000);
            thread::sleep(ten_millis);
        }

        let ten_millis = time::Duration::from_millis(1000);
        thread::sleep(ten_millis);
    }
}
{% endhighlight %}

-->
