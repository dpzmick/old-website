---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on a synthesizer (the kind that makes sounds) in Rust.
This post will be the first of many (see my [Blog Series](/series/) page)

While trying to figure out how to safely send messages between a realtime audio processing thread and other threads (ui thread, disk I/O thread, etc), I stumbled across an excellent talk.
In the talk, the speaker uses `std::shared_ptr` and a lightweight "garbage collector" to easily send messages between threads.
The talk is [on youtube](https://www.youtube.com/watch?v=boPEO2auJj4), I highly recommend watching it.
This post will first explain why such a thing is useful (as does the talk), and how we can do the same thing in Rust.

# Digital audio
Before we can talk about the Rust stuff, we need to understand a bit about digital audio.

To generate audio, audio software sends some digital audio signals to the audio card.
Digital audio signals are just lists of floating point (decimal) numbers.
Think of these numbers as "sound pressure" over time (see [this page](https://docs.cycling74.com/max5/tutorials/msp-tut/mspdigitalaudio.html) for more)

Because sound is continuous, we can't record every possibly value.
Instead, we take measurements of the sound pressure values at some evenly spaced interval.
For CD quality audio, we take 44100 samples per second, or, one sample every 23ish microseconds.
We might sample a sine wave like this (from Wikipedia):

![Samples](https://upload.wikimedia.org/wikipedia/commons/thumb/b/bf/Pcm.svg/500px-Pcm.svg.png)

The audio card turns these lists of samples into some "real-world" audio, which is then played through the speakers.

## Types of audio software
Next let's think about a few different kinds of audio software (this list is by no means complete):

1. Media players (your browser, whatever you listen to music with, a game, etc)
2. Software instruments (think of a virtual piano)
3. Audio plugins (an equalizer in a music player, effects like distortion and compression)
4. Software audio systems

Media players are pretty self explanatory, but the others might need some explanation.
Next on the list is "Software instruments."
These are just pieces of software that can be used to generate sounds.
They are played with external keyboards, or "programmed" with cool user interfaces.

![Drum machine](/img/sound/reason_drums.jpg)
*Drum machine in some audio software*

Next up are audio plugins.
These are pieces of software which take audio as input, transform it in some way, then output the transformed audio.
For example, a graphical equalizer can adjust the volume of different frequency ranges (make the bass louder, make the treble quieter):

![equalizer](/img/sound/itunes_eq.jpg)

Finally, we come to what I'm calling a software audio system.
Because there is only one sound card on your system, any audio you are playing on your computer must be mixed together, then sent to the audio card.
On windows, using the default audio system, I can mix audio with this little mixer thing:

![windows mixer](/img/sound/win_mixer.png)

[Some audio systems](http://www.jackaudio.org/) may also be able to send audio between applications, send [MIDI](https://en.wikipedia.org/wiki/MIDI) signals, keep audio applications in sync, and perform many other tasks.

The software audio system provides a library which other applications use to produce audio.

# Audio system details
Most software audio systems (as far as I know) tend to work the same way.
There is a realtime thread that generates samples and a bunch of other threads that deal with everything else.
The audio thread is usually set up by the audio system's library.
The library calls a user provided callback function to get the samples it needs to deliver to the audio card.

## How fast is realtime?
In the previous section, I claimed that, at 44.1 kHz (the standard CD sample rate), we need to take one audio sample approximately every 23 microseconds.
23 microseconds seems pretty quick, but 192 kHz, a sample must be taken about every 5 microseconds (192 kHz is becoming a bit of an industry standard)!

At these speeds, it would not be possible for the audio system to call our callback function to get every individual sample.
Instead, the audio library system ask us for larger batches of samples.
If we simplify the real world a bit, we can approximate how often our callback function will be called.
Here's a table comparing batch size to the time between callback function calls (all times in milliseconds):

| Batch Size | Time between calls @ 44.1 kHz (millis) | Time between calls @ 192 kHz (millis)
| ---------- | -------------------------------------- | --------------------------------------
| 64         | 1.45                                   | 0.33
| 128        | 2.90                                   | 0.67
| 256        | 5.80                                   | 1.33
| 512        | 11.61                                  | 2.67
| 1024       | 23.22                                  | 5.33
| 2048       | 46.44                                  | 10.67
| 4096       | 92.88                                  | 21.33

There are many complicated trade offs between sample rate/and batch size, so I don't want to get into them now.
You can read [this](http://www.penguinproducer.com/Blog/2011/10/balancing-performance-and-reliability-in-jack/) for a bit more information.
Long story short, use the smallest batch size your computer can handle.

As an audio application developer, we should make sure that our code runs as quickly as possible, even if we have a whole 5 milliseconds to run.
The time we spend is time other audio applications cannot use.
So, even if we theoretically have 5 milliseconds to run, using the entire 5 milliseconds can slow everyone else down.

## Time keeps on ticking
If our callback function fails to generate samples quickly enough (or uses up all of the CPU time), the audio system will produce crackles, pops, and bad sounds.
We call these buffer underruns (or xruns).
**Avoiding buffer underruns must be our top priority!**

Everything we do in our callback function must *always* complete quickly and in a very predictable amount of time.
Unfortunately, this constraint eliminates many things of things we often take for granted, including:

* Synchronization through locking
* Operations with high worst case runtime
* Memory allocation with standard allocators

First, we can't use locks or semaphores or conditional variables or any of those kinds of things inside of our realtime callback function.
If one of our other threads is holding the lock, it might not let go soon enough for us to generate our samples on time!
If you try to make sure you locks will always be released quickly, the scheduler might step in and ruin your plans (this is called [Priority Inversion](https://en.wikipedia.org/wiki/Priority_inversion)).
There are some cases in which it *might* be okay to use locks, but, in general, it is a good idea to avoid them.

Second, we want to avoid operations which have a high worst case runtime.
This can be tricky because some things with bad worst case runtime things have a reasonable [amortized](https://en.wikipedia.org/wiki/Amortized_analysis) runtime.
The canonical example of this is a [dynamic array](https://en.wikipedia.org/wiki/Dynamic_array).
A dynamic array can be inserted into very quickly most of the time, but every so often if must reallocate itself and copy all of its data somewhere else.
For a large array, this expensive copy might cause us to miss our deadline every once and a while.
Fortunately, for some data structures, we can push these worst case costs around and make the operations realtime safe (see [Incremental resizing](https://en.wikipedia.org/wiki/Hash_table#Dynamic_resizing)).

Finally, memory allocation with standard library allocators can cause problems.
Memory allocators are usually thread safe, which usually means that the are locking something.
Additionally, allocation algorithms rarely make any time guarantees; the algorithms they use can have very poor worst case runtimes.
Standard library allocators break both of our other rules!
Luckily, we can still perform dynamic memory allocation if we use [specially designed allocators](http://www.gii.upv.es/tlsf/) or [some pool allocators](https://github.com/supercollider/supercollider/blob/master/common/SC_AllocPool.h) which do not violate our realtime constraints.

I've glossed over many, many details in this section, but, the [cppcon video](https://www.youtube.com/watch?v=boPEO2auJj4), and [this excellent post](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing) both go into much more detail, if you are interested (and why wouldn't you be??).

# Messaging between threads
Suppose we are developing a very simple synthesizer which produces sounds when keys are pressed on a MIDI keyboard.
The audio library we are using delivers all keyboard key presses to the realtime audio callback function at the appropriate times.
The callback function uses a precomputed list of samples to generate sounds when it is told to.
To modify the properties of the sounds that are produced, the user edits the synthesizer settings with a user interface.

The fun starts when we think about how to update the precomputed list of samples when the user changes some properties of sound we are currently generating.
It would be painful (and incorrect) to attempt to handle UI events in the realtime thread, so we will run a UI thread (to handle UI events) and, of course, the realtime audio thread.
The UI thread will not be a realtime thread!
So, whenever the UI thread handles an event which changes the synth settings, the UI thread needs to compute the sample list, then communicate the sample list to the realtime thread.

We can't read and write to the list of samples are the same time (that would be a data race!), so we need some way to control access to the list of samples.
The most obvious way to solve this is, then surround all access to the list of samples with a `mutex`.
With the `mutex`, each thread has exclusive access to the set of samples when it needs to read or write to them, so the threads can never interfere with each other.

Unless you have some aversion to locks (you prefer channels or something), this is probably how most of us would write the application.
Unfortunately, we shouldn't use locks in our realtime thread!

There are many, many solutions to this problem.
I will discuss some others in future posts.
For now, we will just look at one possible solution and discuss some of the tradeoffs we must make.
In a later post, I will compare a different options (with benchmarks), but for this post, we will just write a small example and *think* really hard about it (treat it more like a though experiment).

And, as promised, here is a list of really interesting things you can read to learn more:
* [Overview of Design Patterns for Real-Time Computer Music Systems](http://www.cs.cmu.edu/~rbd/doc/icmc2005workshop/real-time-systems-concepts-design-patterns.pdf)
* [SuperCollider implementation details](http://supercolliderbook.net/rossbencinach26.pdf) from the [SuperCollider book](http://supercolliderbook.net/)
* [Supernova for SuperCollider](http://tim.klingt.org/publications/tim_blechmann_supernova.pdf) a Masters thesis discussing some of these issues


# Reference counted garbage collector
Finally, we can talk about the (intended) subject of this post.
My objective is to send a message containing some samples to the realtime thread.
I think a lock free queue is a pretty good way to send messages between threads, and I like to think [CSP](https://en.wikipedia.org/wiki/Communicating_sequential_processes) style.
I would like to let this message live somewhere on the heap, so that I do not need to copy it multiple times.
If the message lives on the heap, we must ensure two things:

1. It must be freed at some point
2. Multiple threads do not/can not write to the memory at the same time

The specific lock-free queue that we chose to use needs to have a few properties:
* Must be truly lock free
* Must be able to preallocate all of its nodes (cant't allocate and free a node object on a push and pull)

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

```
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
