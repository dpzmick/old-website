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
The synthesizer I program will produce sounds when keys are pressed on a keyboard.
The audio library I am using delivers all keyboard key presses to the realtime audio callback function at the appropriate times.
This means that the callback simply checks if it needs to be making any sounds, then uses a precomputed list of samples to generate a sound.
To modify the properties of the sounds that are produced, the user edits the sound with a user interface.
(note that almost none of this code actually exists yet, so I may write a future post where some of these ideas change)

For this discussion, the threads that we care about are:

1. The realtime audio thread
2. The UI thread

The fun starts when we think about how to update the precomputed list of samples when the user of the synth changes the properties of wave we are currently generating.
Whenever the wave changes, we need to compute the sample list, then communicate the sample list to the realtime thread.

Before we think about the realtime world, lets think about how we might solve this if the application were not realtime.
The most obvious solution is to create an object or structure representing the current set of samples and surround all access to the structure in a mutex.
This way, both threads can lock the set of samples when they need to read or write to them.

Unless you have some aversion to locks (you prefer channels or something), this is probably how most of us would write the application.
Unfortunately, we can't use locks in our realtime thread!

When faced with this problem, I went through a couple of of different solutions before reaching the GC thing I'm going to eventually talk about in this post.

## `memcpy` queue
My first solution was the absolute simplest thing possible.
I set up a fixed sized queue between the two threads.
The queue sent complete lists of samples from one thread to another.
Here's (roughly) how this worked:

* The UI thread recomputes the list of samples on it's stack
* The UI thread shipped the samples to the other thread over the queue (copies the samples into the queue's buffer)
* Every time the realtime callback was called, it checked the queue for new messages.
* If there were new messages, the samples were copied out of the queue's buffer, into the callback's private buffer.

To get this right, we must be careful with the queue that we use.
Let's look at our constraints again.
First, the queue must not use locks to send messages back and forth.
This one we can deal with pretty easily (there are many libraries with lock free queues and ringbuffers).
But, we must be pretty careful about allocation.
We cannot let the queue node get deallocated in the realtime thread (we also don't want to leak the node)
If the queue node is deallocated in the realtime thread, we are making a call to the allocator, but this has been disallowed.
This is not too difficult to work around either, if we are sure to preallocate everything that the queue will need and use atomics to signal when (and where) to read from.
Finally, we should note that it is totally fine for the UI thread to block waiting for space in the queue if the realtime thread is not consuming events fast enough.
This might mean we need to do some event caching on the UI thread side (or maybe pull the latest event off the queue and replace it with a new one or something), but ultimately, it would probably be okay for the UI thread to wait a little while to send the message over the queue.


So, we can probably pull something like this off.
Or can we?
The `memcpy` we must perform will be a slow operation which will only happen every once and a while.
`memcpy` is pretty fast, so we can probably get away with this, if we keep the amount of data sent low, but we have to do `memcpy`s with this technique.
I'm sure we can do better.

## Pointer queue
All problems in computer science can be solved by adding a layer of indirection (or so they say).
We can get rid of all of the copies if we heap allocate everything and send pointers between threads.
In this case, the UI thread allocates some space for the samples, then fills the space in with the new list of samples.
Next, we send this pointer over a bounded, lock free queue.
When there is a message available for the mealtime thread, all it has to do is swap its `current` pointer with the new pointer that came over the queue.
But wait!
What will the realtime thread do with the old list of samples?
It can't free this memory, and we definitely don't want to leak it, so we need to send it to some other thread to be freed (or reused).
What if we just send the memory back to the UI thread, and let the UI thread use the memory for the next event it needs to send us.
This is certainly plausible, but there will be a complications we must work around:
* We cannot use an unbounded queue to send samples from the mealtime thread -> UI thread.
    * As far as I know, there are no unbounded queues which do not perform allocation.
* If the UI thread wants to set up another message before it receives the memory region from the realtime thread, we have two options
    * Wait for the memory to be delivered before sending the next message
    * Allocate another region, send it, then deal with the memory region whenever it gets sent back (free it or save it for later)
* We cannot use a blocking, bounded queue to send the memory back to the UI thread.
    * If the queue is full, we will have to wait until there is room on the queue.
        * Remember that the UI thread is not running in realtime, it might not have been run by the scheduler recently, so the last thing we sent might still be on the queue.
    * We **absolutely cannot** wait!

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
