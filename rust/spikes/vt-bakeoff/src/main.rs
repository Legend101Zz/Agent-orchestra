#![warn(missing_docs)]
//! Reproducible parser bake-off for the Phase-one Bench spike.

use std::hint::black_box;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi;
use termwiz::escape::parser::Parser as TermwizParser;

const ITERATIONS: usize = 400;
const ROWS: usize = 30;
const COLS: usize = 90;

#[derive(Clone, Copy)]
struct Size;

#[derive(Clone, Copy)]
struct Listener;

impl EventListener for Listener {
    fn send_event(&self, _event: Event) {}
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        ROWS
    }

    fn screen_lines(&self) -> usize {
        ROWS
    }

    fn columns(&self) -> usize {
        COLS
    }
}

fn corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(512 * 1024);
    for frame in 0..120 {
        bytes.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[H\x1b[38;2;209;158;77m");
        bytes.extend_from_slice(format!("BENCH frame {frame:03} 世界\r\n").as_bytes());
        for row in 0..28 {
            bytes.extend_from_slice(format!("\x1b[{};1Hrow {row:02} ", row + 2).as_bytes());
            bytes.extend_from_slice(b"\x1b[1mactive\x1b[0m  baton ");
            bytes.extend_from_slice("⠋⠙⠹⠸⠼⠴⠦⠧".as_bytes());
        }
        bytes.extend_from_slice(b"\x1b[?25l");
    }
    bytes
}

fn measure(mut operation: impl FnMut()) -> Duration {
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        operation();
    }
    started.elapsed()
}

fn main() {
    let input = corpus();
    let vt100 = measure(|| {
        let mut parser = vt100::Parser::new(ROWS as u16, COLS as u16, 2_000);
        parser.process(black_box(&input));
        black_box(parser.screen().cell(0, 0));
    });
    let termwiz = measure(|| {
        let mut parser = TermwizParser::new();
        let mut actions = 0_usize;
        parser.parse(black_box(&input), |_| actions += 1);
        black_box(actions);
    });
    let alacritty = measure(|| {
        let mut terminal = Term::new(Config::default(), &Size, Listener);
        let mut parser: ansi::Processor = ansi::Processor::new();
        parser.advance(&mut terminal, black_box(&input));
        black_box(&terminal.grid()[Line(0)][Column(0)]);
    });
    let mib = (input.len() * ITERATIONS) as f64 / (1024.0 * 1024.0);
    println!("parser,elapsed_ms,mib_per_s,screen_replay");
    println!(
        "vt100,{},{:.1},yes",
        vt100.as_millis(),
        mib / vt100.as_secs_f64()
    );
    println!(
        "termwiz,{},{:.1},no",
        termwiz.as_millis(),
        mib / termwiz.as_secs_f64()
    );
    println!(
        "alacritty_terminal,{},{:.1},yes",
        alacritty.as_millis(),
        mib / alacritty.as_secs_f64()
    );
}
