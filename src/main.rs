use std::{
    cmp, fs,
    io::{self, Write},
    os::unix::prelude::FileExt,
    path, sync,
    sync::mpsc::{self, Receiver},
    thread::{self},
    time::Instant,
};

use crossterm::{
    cursor,
    style::{Color, Stylize},
    terminal, ExecutableCommand,
};

use clap::Parser;

struct Config {
    thread_count: u8,
    buffer_size: usize,
}

impl Config {
    pub fn new(thread_count: Option<u8>, buffer_size: Option<usize>) -> Self {
        Config {
            thread_count: thread_count.unwrap_or(1),
            buffer_size: buffer_size.unwrap_or(1024),
        }
    }
}

#[derive(Debug)]
struct Status {
    _timestamp: i64,
    _thread_idx: u8,
    bytes_written: usize,
    offset: u64,
}

impl Status {
    fn new(thread_idx: u8, bytes_written: usize, offset: u64) -> Self {
        Status {
            _timestamp: chrono::offset::Local::now().timestamp_millis(),
            _thread_idx: thread_idx,
            bytes_written,
            offset,
        }
    }
}

/// File copy utility
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    // Source filename
    #[clap()]
    source_filename: String,

    // Target filename
    #[clap()]
    target_filename: String,

    /// Number of threads to use
    #[clap(short, long, default_value_t = 1)]
    thread_count: u8,

    /// Buffer size
    #[clap(short, long, default_value_t = 10000)]
    buffer_size: usize,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let start = Instant::now();
    cp(
        path::Path::new(&args.source_filename),
        path::Path::new(&args.target_filename),
        Config::new(Some(args.thread_count), Some(args.buffer_size)),
    )?;
    let duration = Instant::elapsed(&start).as_millis();
    println!("Done in {duration}ms");
    Ok(())
}

fn cp(source: &path::Path, target: &path::Path, config: Config) -> io::Result<()> {
    let Config {
        buffer_size,
        thread_count,
    } = config;
    let source_file = sync::Arc::new(fs::OpenOptions::new().read(true).open(source)?);
    let source_file_len = source_file.metadata()?.len();
    let target_file = sync::Arc::new(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(target)?,
    );
    let mut join_handles = Vec::with_capacity(thread_count as usize);
    let total_bytes_per_thread = source_file_len.checked_div(thread_count as u64).unwrap();
    let buffer_size = buffer_size.min(total_bytes_per_thread as usize);
    println!("total_bytes_per_thread: {total_bytes_per_thread}");
    println!("Copying {source_file_len} bytes using {thread_count} threads and a {buffer_size} bytes buffer");

    let (tx, rx): (mpsc::Sender<Status>, mpsc::Receiver<Status>) = mpsc::channel();

    for i in 0..thread_count {
        let c_source_file = sync::Arc::clone(&source_file);
        let c_target_file = sync::Arc::clone(&target_file);
        let thread_tx = tx.clone();

        join_handles.push(thread::spawn(move || {
            let mut buffer = vec![0; buffer_size];
            let mut offset = i as u64 * total_bytes_per_thread;
            let last_byte_index_to_read = if i == thread_count - 1 {
                source_file_len
            } else {
                (i as u64 + 1) * total_bytes_per_thread
            };

            loop {
                let bytes_to_read = cmp::min(
                    buffer_size as i64,
                    last_byte_index_to_read as i64 - offset as i64,
                ) as usize;

                if bytes_to_read == 0 {
                    break;
                }

                let bytes_read = c_source_file
                    .read_at(&mut buffer[0..bytes_to_read], offset)
                    .unwrap();

                if bytes_read == 0 {
                    break;
                }

                let bytes_written = c_target_file
                    .write_at(&buffer[0..bytes_read], offset)
                    .unwrap();

                thread_tx
                    .send(Status::new(i, bytes_written, offset))
                    .unwrap();

                offset += bytes_read as u64;
            }
        }));
    }

    drop(tx);
    report_status(rx, source_file_len)?;

    for jh in join_handles {
        let _ = jh.join();
    }

    Ok(())
}

fn report_status(rx: Receiver<Status>, source_file_len: u64) -> io::Result<()> {
    const BLOCK: &str = "\u{2596}";

    let terminal_col_count = terminal::size()?.0;
    let block = BLOCK.with(Color::DarkGreen);
    io::stdout().execute(cursor::Hide)?;
    io::stdout().execute(cursor::SavePosition)?;

    let (col, row) = cursor::position()?;

    for status in rx {
        let col = col
            + (terminal_col_count as f64 * (status.offset as f64 / source_file_len as f64)) as u16;
        let count = (terminal_col_count as f64
            * (status.bytes_written as f64 / source_file_len as f64)) as usize;
        let count = count.max(1);

        io::stdout().execute(crossterm::cursor::MoveTo(col, row))?;
        for _ in 0..count {
            print!("{block}");
        }
        io::stdout().flush()?;
    }

    io::stdout().execute(crossterm::cursor::RestorePosition)?;
    println!();
    io::stdout().execute(cursor::Show)?;

    Ok(())
}
