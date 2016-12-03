---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on a synthesizer (the kind that makes sounds) in Rust.
While trying to figure out how to safely send messages between a realtime audio processing thread and other threads (ui thread, disk I/O thread, etc), I stumbled across an excellent talk.
In the talk, an example is given using `std::shared_ptr` and a lightweight "garbage collector" to easily send messages between threads.
The talk is [on youtube](https://www.youtube.com/watch?v=boPEO2auJj4), I highly recommend watching it.
This post will first explain why such a thing is useful (as does the talk), and how I've used this trick in Rust.

# Digital audio
Before we can talk about the Rust stuff, we need to understand a bit about digital audio (feel free to skip this section).

To generate audio, audio software sends some digital audio signals to the audio card.
Digital audio signals are just lists of floating point (decimal) numbers.
Think of these numbers as "sound pressure" over time (see [this page](https://docs.cycling74.com/max5/tutorials/msp-tut/mspdigitalaudio.html) for more)

Because sound is continuous, we can't record every possibly value.
Instead, we take measurements of the sound pressure values at some evenly spaced interval.
For CD quality audio, we take 44100 samples per second, or, one sample every 23ish nanoseconds.
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
On windows, using the out of the box audio system, I can mix audio with this little mixer thing:

![windows mixer](/img/sound/win_mixer.png)

[Some audio systems](http://www.jackaudio.org/) may also be able to send audio between applications, send [MIDI](https://en.wikipedia.org/wiki/MIDI) signals, keep audio applications in sync, and perform many other tasks.

The software audio system provides a library which other applications use to produce audio.

# Time waits for no man
Most software audio systems (as far as I know) tend to work the same way.
There is a realtime thread that generates samples, and a bunch of other threads that deal with everything else.
The audio thread is usually set up by the audio system's library.
The library calls a user provided callback function to get the next batch of samples it needs to deliver to the audio card.

If the callback function fails to generate samples quickly enough, the audio system will produce crackles, pops, and bad sounds.
We call these buffer underruns (or xruns).
**Avoiding buffer underruns must be our top priority!**

Everything we do in our callback function must *always* complete quickly and in a very predictable amount of time.
Unfortunately, this constraint eliminates many things of things we often take for granted, including:

* Synchronization through locking
* Memory allocation with standard allocators

First, we can't use locks or semaphores or conditional variables or any of those kinds of things inside of our realtime callback function.
If one of our other threads is holding the lock, it might not let go soon enough for us to generate our samples on time!
If you are smart enough to work out [all](https://en.wikipedia.org/wiki/Priority_inversion) [of](http://lists.apple.com/archives/Coreaudio-api/2001/May/msg00032.html) [these](https://en.wikipedia.org/wiki/Deadlock) [details](http://stackoverflow.com/a/4296991), you can maybe manage to use locks in your realtime thread, but I'm not that smart.

Second, memory allocation with standard library allocators can cause us some problems.
These memory allocators are usually thread safe, which probably implies that the are locking something, and allocation algorithms rarely make any time guarantees.
If you want to write your own allocator (controlling all of the details) you may be able to use it, but we aren't going to have much luck with the standard allocators.

I've glossed over many, many details in this section (this post is getting long), but, the [cppcon video](https://www.youtube.com/watch?v=boPEO2auJj4), and [this excellent post](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing) both cover more details, if you are interested.

# Messaging between threads
The synthesizer program will produce sounds when keys are pressed on a keyboard.
The audio library I am using delivers all keyboard key presses to the realtime audio callback function at the appropriate times.
The callback function uses a precomputed list of samples to generate sounds when it is told to.
To modify the properties of the sounds that are produced, the user edits the synthesizer settings with a user interface.
(note that almost none of this code actually exists yet, so I may write a future post where some of these ideas change)

The fun starts when we think about how to update the precomputed list of samples when the user of the synth changes the properties of wave we are currently generating.
It would be painful to attempt to handle UI events in the realtime thread, so we will run a UI thread (to handle UI events) and, of course, the realtime audio thread.
The UI thread will not be a realtime thread!
So, whenever the UI thread handles an event which changes the synth settings, the UI thread needs to compute the sample list, then communicate the sample list to the realtime thread.

We can't read and write to the list of samples are the same time (that would be a data race!), so we need some way to control access to the list of samples.
The most obvious way to solve this is, then surround all access to the list of samples with a `mutex`.
With the `mutex`, each thread has exclusive access to the set of samples when it needs to read or write to them, so the threads can never interfere with each other.

Unless you have some aversion to locks (you prefer channels or something), this is probably how most of us would write the application.
Unfortunately, we shouldn't use locks in our realtime thread!

When faced with this problem, I worked through a couple of of different "solutions" before reaching the GC thing I'm going to eventually talk about in this post.
Let's walk though some of them together.

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

<!--

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
