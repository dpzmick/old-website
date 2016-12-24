#![feature(arc_counts)]

use std::thread;
use std::f32;
use std::sync::{Arc, Mutex, mpsc};

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

// beginning of GC implementation
// struct TrustMe<T> {
//     pub data: T
// }

// unsafe impl<T> Send for TrustMe<T> {}

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

// impl<T: Send + 'static> Drop for GC<T> {
//     fn drop(&mut self) {
//         println!("collector going down!");
//         self.notify.send(true).unwrap();

//         let t = self.thread.take();
//         match t {
//             Some(t) => t.join().unwrap(),
//             None    => ()
//         }
//     }
// }
// end of GC implementation

enum Message {
    NewSamples(Arc<Samples>),
    Shutdown,
}

/// A struct containing the realtime callback and all data owned by the realtime thread
struct RealtimeThread {
    current_samples: Option<Arc<Samples>>,
    incoming:        mpsc::Receiver<Message>,
}

impl RealtimeThread {
    fn new(incoming: mpsc::Receiver<Message>) -> Self {
        RealtimeThread {
            current_samples: None,
            incoming:        incoming,
        }
    }

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
}

/// A struct which runs the UI thread and contains all of the data owned by the UI thread
struct UIThread {
    outgoing: mpsc::SyncSender<Message>,
}

impl UIThread {
    fn new(outgoing: mpsc::SyncSender<Message>) -> Self {
        UIThread { outgoing: outgoing }
    }

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

    /// All of the UI thread code
    fn run(&mut self) {
        // start the garbage collector
        // let mut gc = GC::new();

        // create 10 "ui events"
        for i in 0..5 {
            let volume = i as f32 / 10.0;
            let samples = Arc::new(self.compute_samples(volume));
            // gc.track(samples.clone());

            // send the samples to the other thread
            println!("[ui] sending new samples. Second sample: {}", samples[1]);
            self.outgoing.send(Message::NewSamples(samples)).unwrap();
        }

        // tell the other thread to shutdown
        self.outgoing.send(Message::Shutdown).unwrap();
    }
}

fn main() {
    let (tx, rx) = mpsc::sync_channel(0);
    let rt = RealtimeThread::new(rx);
    let ui = UIThread::new(tx);
    run_threads(rt, ui);
}
