extern crate cpal;
extern crate crossterm;
extern crate ringbuf;
extern crate hound;
extern crate chrono;

use cpal::{traits::{DeviceTrait, HostTrait, StreamTrait}, StreamConfig};
use crossterm::{event, ExecutableCommand};
use hound::{WavSpec, WavWriter};
use std::sync::{atomic::AtomicBool, atomic::Ordering, mpsc::channel, Arc};
use std::thread;
use chrono::Local;
use std::error::Error;

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u16 = 1;
const CACHE_SIZE_IN_BYTES: usize = 512 * 1024 * 1024; // 512 MB
const CACHE_FLUSH_SIZE: usize = CACHE_SIZE_IN_BYTES / 2; // Half the cache size

fn main() -> Result<(), Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device available");

    let mut max_sample_rate: u32 = 0;
    // Ausgabe der unterstützten Formate
    let supported_formats= device.supported_input_configs()?;
    for config_range  in supported_formats {
        println!("Supported format: {:?}", config_range );
        if config_range.max_sample_rate().0 > max_sample_rate {
            max_sample_rate = config_range.max_sample_rate().0;
        }
    }

    // Verwenden Sie die bevorzugte Samplerate, wenn sie verfügbar ist, sonst die höchstmögliche
    let selected_sample_rate = if max_sample_rate >= cpal::SampleRate(SAMPLE_RATE).0 {
        cpal::SampleRate(SAMPLE_RATE).0
    } else {
        max_sample_rate
    };

    let config = StreamConfig {
        channels: CHANNELS,
        sample_rate: cpal::SampleRate(selected_sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };
    
    let (sender, receiver) = channel();

    // Thread for caching samples
    let cache_sender = sender.clone();
    thread::spawn(move || {
        println!("Record start");

        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut samples = Vec::with_capacity(data.len());
                samples.extend_from_slice(data);
                cache_sender.send(samples).unwrap();
            },
            |err| eprintln!("Error during stream: {:?}", err),
            None,
        ).unwrap();
        stream.play().unwrap();
    });

    // Flag für das saubere Beenden des Programms
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Thread für das Erkennen von Tastendrücken
    thread::spawn(move || {
        while r.load(Ordering::SeqCst) {
            if event::poll(std::time::Duration::from_millis(100)).unwrap() {
                if let Ok(true) = event::read().map(|e| matches!(e, event::Event::Key(_))) {
                    r.store(false, Ordering::SeqCst);
                }
            }
        }
    });

    // Thread for writing to WAV files
    let write_thread = thread::spawn(move || {
        let mut cached_samples = Vec::new();

        println!("wave cache start");

        loop {
            if let Ok(samples) = receiver.try_recv() {
                cached_samples.extend(samples);

                if cached_samples.len() * std::mem::size_of::<f32>() >= CACHE_FLUSH_SIZE || !running.load(Ordering::SeqCst) {
                    let filename = format!("{}.wav", Local::now().format("%Y-%m-%d_%H-%M-%S"));
                    println!("File {}", filename);
                    let spec = WavSpec {
                        channels: CHANNELS,
                        sample_rate: selected_sample_rate,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    };
                    let mut writer = WavWriter::create(filename, spec).unwrap();

                    for sample in cached_samples.drain(..) {
                        writer.write_sample(sample).unwrap();
                    }
                    writer.finalize().unwrap();

                    if !running.load(Ordering::SeqCst) {
                        break;
                    }

                    println!("finished file");
                }
            }
        }
    });

    write_thread.join().unwrap();
        
    println!("Record stop");

    Ok(())
}
