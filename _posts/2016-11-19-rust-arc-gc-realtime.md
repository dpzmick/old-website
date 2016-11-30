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
My rust synthesizer application will have a variety of threads running at all times.
The three threads that we care about are:

1. The realtime audio thread
2. The UI thread

The audio library I am using treats MIDI events (such as, play a note starting at this time and ending at this time) as input events to the realtime thread.
This means, the realtime thread callback code needs to know the following things:

1. What notes it is currently playing (since a note may start in one callback and finish in the other)
2. What samples to send when a note is played

Let's think about the second of these two issues.

Suppose we have a simple synthesizer which can generate only two different kinds of sounds.
Let's call one a square wave and the other a sine wave.
Additionally, there are a variety of little tweaks which we can make to the sound.
For example, we might be able to skew the wave slightly, adjust the pitch slightly, and some sort of decay to the notes, etc.

The problem is, the user of the synth will change properties of wave we are currently generating with the UI.
Whenever the wave changes, we need to communicate to the realtime thread that we now have a new type of wave to deal with.

Before we think about the realtime world, lets think about how we might solve this if the world were not realtime.
The most obvious solution that any college graduate of computer science might come up with is to create an object or structure representing the current set of samples and surround all access to the structure in a mutex.
This way, whenever the UI needs to update things, it waits for the sample generator to be available, tweaks it, then gives it back.

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
