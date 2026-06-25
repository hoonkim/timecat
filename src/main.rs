use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use chrono::Local;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

const DIGIT_HEIGHT: usize = 7;
const DIGIT_WIDTH: usize = 5;
const DIGIT_GAP: usize = 1;
const MIN_MARGIN: usize = 1;

const DIGITS: [[&str; DIGIT_HEIGHT]; 10] = [
    [
        "#####", "#   #", "#   #", "#   #", "#   #", "#   #", "#####",
    ],
    [
        "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", "#####",
    ],
    [
        "#####", "    #", "    #", "#####", "#    ", "#    ", "#####",
    ],
    [
        "#####", "    #", "    #", "#####", "    #", "    #", "#####",
    ],
    [
        "#   #", "#   #", "#   #", "#####", "    #", "    #", "    #",
    ],
    [
        "#####", "#    ", "#    ", "#####", "    #", "    #", "#####",
    ],
    [
        "#####", "#    ", "#    ", "#####", "#   #", "#   #", "#####",
    ],
    [
        "#####", "    #", "    #", "   # ", "  #  ", " #   ", "#    ",
    ],
    [
        "#####", "#   #", "#   #", "#####", "#   #", "#   #", "#####",
    ],
    [
        "#####", "#   #", "#   #", "#####", "    #", "    #", "#####",
    ],
];

const COLON: [&str; DIGIT_HEIGHT] = [" ", "#", "#", " ", "#", "#", " "];

fn main() -> io::Result<()> {
    let mut terminal = TerminalSession::start()?;
    run(&mut terminal.stdout)
}

fn run(stdout: &mut io::Stdout) -> io::Result<()> {
    let mut last_drawn = String::new();

    loop {
        let now = Local::now().format("%H:%M:%S").to_string();

        if now != last_drawn {
            draw_clock(stdout, &now)?;
            last_drawn = now;
        }

        let tick_start = Instant::now();
        while tick_start.elapsed() < Duration::from_millis(200) {
            if event::poll(Duration::from_millis(25))? && should_quit(event::read()?) {
                return Ok(());
            }
        }
    }
}

fn draw_clock(stdout: &mut io::Stdout, time: &str) -> io::Result<()> {
    let (terminal_width, terminal_height) = terminal::size()?;
    let scale = clock_scale(time, terminal_width as usize, terminal_height as usize);
    let lines = render_digits(time, scale);
    let content_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let content_height = lines.len();

    queue!(
        stdout,
        SetBackgroundColor(Color::Black),
        SetForegroundColor(Color::White),
        Clear(ClearType::All)
    )?;

    if content_width > terminal_width as usize || content_height > terminal_height as usize {
        draw_too_small(stdout, terminal_width, terminal_height)?;
        stdout.flush()?;
        return Ok(());
    }

    let digit_x = ((terminal_width as usize - content_width) / 2) as u16;
    let digit_y = ((terminal_height as usize - content_height) / 2) as u16;

    for (row, line) in lines.iter().enumerate() {
        queue!(stdout, MoveTo(digit_x, digit_y + row as u16), Print(line))?;
    }

    stdout.flush()
}

fn clock_scale(time: &str, terminal_width: usize, terminal_height: usize) -> usize {
    let base_width = base_clock_width(time);
    let available_width = terminal_width.saturating_sub(MIN_MARGIN * 2);
    let available_height = terminal_height.saturating_sub(MIN_MARGIN * 2);
    let width_scale = available_width / base_width.max(1);
    let height_scale = available_height / DIGIT_HEIGHT;

    width_scale.min(height_scale).max(1)
}

fn base_clock_width(time: &str) -> usize {
    let glyph_width: usize = time
        .chars()
        .map(|ch| if ch == ':' { 1 } else { DIGIT_WIDTH })
        .sum();
    let gaps = time.chars().count().saturating_sub(1) * DIGIT_GAP;

    glyph_width + gaps
}

fn render_digits(time: &str, scale: usize) -> Vec<String> {
    let mut rows = vec![String::new(); DIGIT_HEIGHT];

    for (index, ch) in time.chars().enumerate() {
        if index > 0 {
            append_gap(&mut rows, DIGIT_GAP * scale);
        }

        match ch {
            '0'..='9' => {
                let digit = ch.to_digit(10).expect("digit") as usize;
                append_pattern(&mut rows, &DIGITS[digit], scale);
            }
            ':' => append_pattern(&mut rows, &COLON, scale),
            _ => {}
        }
    }

    scale_rows(rows, scale)
}

fn append_pattern(rows: &mut [String], pattern: &[&str; DIGIT_HEIGHT], scale: usize) {
    for (row, segment_row) in rows.iter_mut().zip(pattern) {
        for ch in segment_row.chars() {
            let pixel = if ch == '#' { '█' } else { ' ' };
            row.push_str(&pixel.to_string().repeat(scale));
        }
    }
}

fn append_gap(rows: &mut [String], width: usize) {
    for row in rows {
        row.push_str(&" ".repeat(width));
    }
}

fn scale_rows(rows: Vec<String>, scale: usize) -> Vec<String> {
    rows.into_iter()
        .flat_map(|row| std::iter::repeat_n(row, scale))
        .collect()
}

fn draw_too_small(
    stdout: &mut io::Stdout,
    terminal_width: u16,
    terminal_height: u16,
) -> io::Result<()> {
    let message = format!(
        "Terminal too small: current {}x{}",
        terminal_width, terminal_height
    );
    let x = terminal_width.saturating_sub(message.len() as u16) / 2;
    let y = terminal_height / 2;

    queue!(
        stdout,
        MoveTo(x, y),
        SetForegroundColor(Color::White),
        Print(message),
        ResetColor
    )
}

fn should_quit(event: Event) -> bool {
    match event {
        Event::Key(key) if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') => true,
        Event::Key(key)
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            true
        }
        _ => false,
    }
}

struct TerminalSession {
    stdout: io::Stdout,
}

impl TerminalSession {
    fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, LeaveAlternateScreen, ResetColor);
        let _ = disable_raw_mode();
    }
}
