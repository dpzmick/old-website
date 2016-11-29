---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on developing a synthesizer (the kind that makes sounds) in Rust.
While trying to figure out how to safely send messages between the realtime audio processing thread and other threads (ui thread, disk I/O thread, etc), I stumbled across an excellent talk which validated my attempt to use reference counting to help ease the situation.
The talk is [on youtube](https://www.youtube.com/watch?v=boPEO2auJj4), I highly recommend it.
This post will first motivate the value of a very simple garbage collector in these kinds of applications, and will explain the implementation.

# Digital audio
<!---
Before delving into the details of audio programming, we need to understand a bit about how computers deal with sound.
I don't want to go too far in depth (watch the video), but I will give a short and sweet overview here.
In the "real world," sound/audio is just a vibration of some medium (air, water, whatever) that our ears and brain interpret as sound.
One way to visualize these waves is "pressure over time," where pressure is the amount of vibration hitting some receiver (the ear) at some instant.
For a pure sine wave (the simplest sound), sound pressure over looks like this:

![Sine wave](/img/sound/sine.png)

TODO add some example sounds

Since this curve is smooth and continuous (has an infinite number of points), we can't exactly represent it in a computer.
Instead, we measure the current value of the curve at regular intervals.
Then, when replaying the audio.
-->

Digital audio is complex.
If you would like a more compressive explanation, check out the video linked above.

Computers think of audio as long arrays of floating point numbers (in the simplest case).
Each one of these floats is called a sample because it is a measurement of the sound wave which some recording device observed at a given time (it took a sample of the wave).
These samples are evenly spaced.
To record and play back digital audio samples, we use Analog (sound waves) <-> Digital converters.

ADD DIAGRAM HERE

Because sound waves are a continuous thing in nature, we can'hert just directly record what we hear in the real world, store it on a disk somewhere, and replay it (by disk I mean digital disk, since this is exactly how vinyl records work).
Instead, we store "samples" of the real audio data.
TODO actually finish explaining this with diagrams and stuff
TODO what are plugins
TODO what does the whole ecosystem look like?
TODO should this be a separate post?

# The event loop waits for no one
Computer audio systems are complicated, but, if we take all of the audio drivers, OS support, and audio library implementation details as a black box, audio code has some pretty simple properties.
Almost all audio libraries share a pretty simple interface: The user provides an audio processing function, and the audio library calls the function at all the right times.
Each call to this function represents a fragment in time.
This magical audio function often has two responsibilities:

1. Handle any incoming audio (and midi, depending on the library) events which are associated with the current fragment in time
2. Generate the needed output samples (and maybe midi signals) for this slice in time.

Each time the audio library calls our callback function, we must provide our output samples quickly enough.
If we fail to meet the deadline for the audio library, we will cause "x-runs"/"glitches"/"annoying pops", which basically means that the audio card will emit some really unpleasant sound.
If the audio card is hooked to a large PA system, these sorts of errors could damage speakers, or make a concert very unpleasant for lots of people.
In non-live settings, badly behaved audio applications can still be very, very, very annoying to their users.

This means that, inside of our realtime audio code (the audio event function), we need to make sure we never do anything that might cause us to miss our deadline.
Unfortunately, many things we typically take for granted have behavior which sometimes would cause us to miss the deadline:

## Synchronization through locking
## Memory allocation
## Amortized reallocations
This is usually so it is eliminated anyway but lets discuss anyway


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
