---
layout: post
title: "\"Garbage collection\" for Rust Arc Pointers"
summary: "Cleaning up after yourself in realtime"
---

Recently, I've been working on developing a synthesizer (the kind that makes sounds) in Rust.
While trying to figure out how to safely send messages between the realtime audio processing thread and other threads (ui thread, disk I/O thread, etc), I stumbled across an excellent talk which validated my attempt to use reference counting to help ease the situation.
The talk is [on youtube](https://www.youtube.com/watch?v=boPEO2auJj4), I highly recommend it.
This post will first explain why I've done what I've done, then I will explain how I'v done it.

# Digital audio
Before diving in to the meat of this post, let me first give a quick overview of digital audio (if you know this stuff, skip this section!)
For the sake of this post, we will only consider cases where a computer generates audio (we are ignoring recording).
There are a few important pieces of this equation:

1. An audio software system (creates digital audio signals)
2. A hardware audio card (converts digital audio signals into sound)
3. Some sort of speakers (these are of course only needed if you actually want to hear anything)

To generate audio, the software audio system sends some "samples" to the audio card (more on these in a second).
The audio card turns these audio samples into some real audio.
Since we don't really need to understand sampling to get through this post, lets just gloss over it quickly enough to get to the content.

Computers think of audio as long lists of floating point (decimal) numbers.
These floating point numbers are "sound pressure" over time (see [this page](https://docs.cycling74.com/max5/tutorials/msp-tut/mspdigitalaudio.html) for more)
Because sound is continuous, we can't record every possibly value.
Instead, we take measurments of the values at some evenly spaced interval (eg 44100 samples per second, or, one sample every 23ish nanoseconds).
For example (from wikipedia):

![Samples](https://upload.wikimedia.org/wikipedia/commons/thumb/b/bf/Pcm.svg/500px-Pcm.svg.png)

The audio driver takes the samples down into the depths of the hardware and eventually produces sound using some magic I don't really understand.

## Types of audio software
Okay, so audio software just generates a bunch of numbers and sends them to some magically audio driver the operating system provides, now we need to think about a few different kinds of audio software (this list is by no means complete):

1. Media players (your browser, whatever you listen to music with, a game perhaps, etc)
2. Software instruments (think of a virtual piano that makes sounds)
3. Audio plugins (an equalizer in a music player, effects like distortion/compression/equalization for software instruments)
4. Software audio subsystems

Media players are pretty self explanatory, but the others might need some explanation.
Let's start with the next easiest: software instruments.
These are just pieces of software that can be used to generate sounds.
Sometimes these are played with external keyboards, and sometimes they are "programmed" with cool user interfaces.

![Drum machine](/img/sound/reason_drums.jpg)
*Drum machine in some audio software*

Next we come to audio plugins.
These are pieces of software which take audio as input, transform it in some way, then output the new audio.
The most familar of these is probably the graphical equalizer, which allows a user to adjust the volume of different frequency ranges (make the bass louder, make the treble quieter):

![equalizer](/img/sound/itunes_eq.jpg)

Finally, we come to the hardest to understand, what I'm calling a software audio subsystem.
Because there is only one sound card on your system, any audio you are playing on your computer must be mixed together, then sent to the audio card.
Different operating systems have different pieces of software to perform this task, and most (except OS X) have a few different sets of drivers and mixing software.
On windows, using the out of the box audio system, I can control levels with this little mixer thing:

![windows mixer](/img/sound/win_mixer.png)

Okay, so all of this doesn't seem so bad.
what makes audio software complicated?
One word: timing.
Many pieces of audio software and hardware work together to produce audio.
If that audio isn't produced at the right time, the music you are listening to, movie you are watching, game you are playing, instrument you are playing, etc, will have loud pops and crackles and various other unpleasantries.

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
