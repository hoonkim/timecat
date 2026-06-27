use std::{
    io::{self, Write},
    process::Command,
    time::{Duration, Instant},
};

use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDateTime, TimeZone};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DIGIT_HEIGHT: usize = 7;
const DIGIT_WIDTH: usize = 5;
const DIGIT_GAP: usize = 1;
const MIN_MARGIN: usize = 1;
const MAX_CLOCK_SCALE: usize = 12;
const MAX_KITTY_TEXT_SCALE: usize = 7;
const CALENDAR_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const ALERT_BEFORE_START: Duration = Duration::from_secs(10 * 60);
const ALERT_GRACE_PERIOD: Duration = Duration::from_secs(60);
const ALERT_FLASH_INTERVAL: Duration = Duration::from_millis(220);

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
    if handle_cli_args() {
        return Ok(());
    }

    let mut terminal = TerminalSession::start()?;
    run(&mut terminal.stdout)
}

fn handle_cli_args() -> bool {
    let Some(arg) = std::env::args().nth(1) else {
        return false;
    };

    match arg.as_str() {
        "-h" | "--help" => {
            println!("timecat {VERSION}");
            println!("A large 7-segment digital clock for modern terminals.");
            println!();
            println!("Usage: timecat");
            println!();
            println!("Controls:");
            println!("  q, Ctrl-C         quit");
            println!("  Esc, click        dismiss calendar alert");
            println!();
            println!("Calendar:");
            println!("  Run gws auth login with Calendar readonly scope.");
            println!("  Or install and authorize gcalcli to show Google Calendar events.");
            println!("  Or set TIMECAT_CALENDAR_CMD to a command that prints TSV events.");
            println!(
                "  TSV formats: start<TAB>end<TAB>title or date<TAB>time<TAB>end-date<TAB>end-time<TAB>title."
            );
            true
        }
        "-V" | "--version" => {
            println!("timecat {VERSION}");
            true
        }
        _ => false,
    }
}

fn run(stdout: &mut io::Stdout) -> io::Result<()> {
    let mut last_drawn = String::new();
    let mut last_terminal_size = None;
    let mut calendar = CalendarState::new();
    let mut alert = AlertState::new();

    loop {
        let now_dt = Local::now();
        calendar.refresh_if_due(now_dt);
        alert.update(now_dt, calendar.alert_candidate(now_dt));

        let now = now_dt.format("%H:%M:%S").to_string();
        let terminal_size = terminal::size()?;
        let render_key = format!(
            "{now}|{}|{}",
            calendar.render_key(now_dt),
            alert.render_key()
        );

        if render_key != last_drawn || Some(terminal_size) != last_terminal_size {
            draw_clock(stdout, &now, &calendar, &alert, now_dt)?;
            last_drawn = render_key;
            last_terminal_size = Some(terminal_size);
        }

        let tick_start = Instant::now();
        while tick_start.elapsed() < Duration::from_millis(200) {
            if event::poll(Duration::from_millis(25))? {
                match handle_event(event::read()?, &mut alert) {
                    InputAction::Quit => return Ok(()),
                    InputAction::Dismissed => {
                        last_drawn.clear();
                        break;
                    }
                    InputAction::None => {}
                }
            }
        }
    }
}

fn draw_clock(
    stdout: &mut io::Stdout,
    time: &str,
    calendar: &CalendarState,
    alert: &AlertState,
    now: DateTime<Local>,
) -> io::Result<()> {
    let (terminal_width, terminal_height) = terminal::size()?;
    let schedule_lines = calendar.render_lines(now, terminal_width as usize);
    let scale = clock_scale(
        time,
        terminal_width as usize,
        terminal_height as usize,
        schedule_lines.len(),
        alert,
    );
    let lines = render_digits(time, scale);
    let content_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let content_height = lines.len();

    let background = alert.background_color();
    let foreground = alert.foreground_color();
    let mut frame = vec![Vec::new(); terminal_height as usize];

    let Some(layout) = clock_layout(
        content_width,
        content_height,
        scale,
        schedule_lines.len(),
        terminal_width,
        terminal_height,
        alert,
    ) else {
        if is_kitty_terminal() {
            add_kitty_compact_time_fragment(&mut frame, time, terminal_width, terminal_height);
        } else {
            add_compact_time_fragment(&mut frame, time, terminal_width, terminal_height);
        }
        add_schedule_fragments(
            &mut frame,
            &schedule_lines,
            terminal_width,
            terminal_height,
            compact_schedule_y(schedule_lines.len(), terminal_height, alert),
            1,
        );
        add_alert_message_fragment(&mut frame, alert, terminal_width, terminal_height);
        return draw_frame(stdout, &frame, terminal_width, background, foreground);
    };

    for (row, line) in lines.iter().enumerate() {
        add_fragment(
            &mut frame,
            layout.digit_y + row as u16,
            layout.digit_x,
            line.clone(),
            line.chars().count(),
            foreground,
            None,
        );
    }

    add_schedule_fragments(
        &mut frame,
        &schedule_lines,
        terminal_width,
        terminal_height,
        layout.schedule_y,
        layout.schedule_text_scale,
    );
    add_alert_message_fragment(&mut frame, alert, terminal_width, terminal_height);
    draw_frame(stdout, &frame, terminal_width, background, foreground)
}

#[derive(Clone)]
struct TextFragment {
    x: u16,
    text: String,
    cell_width: usize,
    foreground: Color,
    background: Option<Color>,
}

fn add_fragment(
    frame: &mut [Vec<TextFragment>],
    y: u16,
    x: u16,
    text: String,
    cell_width: usize,
    foreground: Color,
    background: Option<Color>,
) {
    let Some(row) = frame.get_mut(y as usize) else {
        return;
    };
    if cell_width == 0 {
        return;
    }

    row.push(TextFragment {
        x,
        text,
        cell_width,
        foreground,
        background,
    });
}

fn draw_frame(
    stdout: &mut io::Stdout,
    frame: &[Vec<TextFragment>],
    terminal_width: u16,
    background: Color,
    foreground: Color,
) -> io::Result<()> {
    let width = terminal_width as usize;

    for (y, fragments) in frame.iter().enumerate() {
        queue!(
            stdout,
            MoveTo(0, y as u16),
            SetForegroundColor(foreground),
            SetBackgroundColor(background)
        )?;

        let mut cursor = 0usize;
        let mut fragments = fragments.clone();
        fragments.sort_by_key(|fragment| fragment.x);

        for fragment in fragments {
            let x = fragment.x as usize;
            if x >= width {
                continue;
            }
            if x > cursor {
                queue!(stdout, Print(" ".repeat(x - cursor)))?;
                cursor = x;
            }

            queue!(
                stdout,
                SetForegroundColor(fragment.foreground),
                SetBackgroundColor(fragment.background.unwrap_or(background)),
                Print(fragment.text),
                SetForegroundColor(foreground),
                SetBackgroundColor(background)
            )?;
            cursor = cursor.saturating_add(fragment.cell_width).min(width);
        }

        if cursor < width {
            queue!(stdout, Print(" ".repeat(width - cursor)))?;
        }
    }

    queue!(stdout, ResetColor)?;
    stdout.flush()
}

fn add_schedule_fragments(
    frame: &mut [Vec<TextFragment>],
    lines: &[String],
    terminal_width: u16,
    terminal_height: u16,
    start_y: u16,
    text_scale: usize,
) {
    if lines.is_empty() || terminal_width == 0 || terminal_height == 0 {
        return;
    }

    let text_scale = text_scale.max(1);
    let available_width = (terminal_width as usize / text_scale).max(1);
    for (index, line) in lines.iter().enumerate() {
        let text = truncate_to_width(line, available_width);
        let x = centered_x_scaled(&text, terminal_width, text_scale);
        let y = start_y.saturating_add((index * text_scale) as u16);
        let rendered_text = if is_kitty_terminal() && text_scale > 1 {
            format!("\x1b]66;s={text_scale};{text}\x07")
        } else {
            text.clone()
        };
        add_fragment(
            frame,
            y,
            x,
            rendered_text,
            display_width(&text) * text_scale,
            Color::Grey,
            None,
        );
    }
}

fn compact_schedule_y(line_count: usize, terminal_height: u16, alert: &AlertState) -> u16 {
    let alert_reserved = if alert.is_active() { 3 } else { 1 };
    terminal_height.saturating_sub(line_count as u16 + alert_reserved)
}

fn add_alert_message_fragment(
    frame: &mut [Vec<TextFragment>],
    alert: &AlertState,
    terminal_width: u16,
    terminal_height: u16,
) {
    let Some(message) = alert.message() else {
        return;
    };
    if terminal_width == 0 || terminal_height < 2 {
        return;
    }

    let text = truncate_to_width(&message, terminal_width as usize);
    let x = centered_x(&text, terminal_width);
    let y = terminal_height.saturating_sub(2);

    add_fragment(
        frame,
        y,
        x,
        text.clone(),
        display_width(&text),
        Color::Black,
        Some(Color::Yellow),
    );
}

struct ClockLayout {
    digit_x: u16,
    digit_y: u16,
    schedule_y: u16,
    schedule_text_scale: usize,
}

fn clock_layout(
    content_width: usize,
    content_height: usize,
    scale: usize,
    schedule_line_count: usize,
    terminal_width: u16,
    terminal_height: u16,
    alert: &AlertState,
) -> Option<ClockLayout> {
    let usable_height = usable_content_height(terminal_height as usize, alert);
    let schedule_text_scale = schedule_text_scale(scale);
    let block_height =
        content_height + schedule_block_height(schedule_line_count, scale, schedule_text_scale);

    if content_width > terminal_width as usize || block_height > usable_height {
        return None;
    }

    let digit_x = ((terminal_width as usize - content_width) / 2) as u16;
    let digit_y = ((usable_height - block_height) / 2) as u16;
    let schedule_y = if schedule_line_count == 0 {
        0
    } else {
        digit_y + content_height as u16 + schedule_gap(scale) as u16
    };

    Some(ClockLayout {
        digit_x,
        digit_y,
        schedule_y,
        schedule_text_scale,
    })
}

fn clock_scale(
    time: &str,
    terminal_width: usize,
    terminal_height: usize,
    schedule_line_count: usize,
    alert: &AlertState,
) -> usize {
    let base_width = base_clock_width(time);
    let available_width = terminal_width.saturating_sub(MIN_MARGIN * 4);
    let available_height = usable_content_height(terminal_height, alert).saturating_sub(MIN_MARGIN);
    let max_scale = (available_width / base_width.max(1))
        .min(available_height / DIGIT_HEIGHT)
        .min(MAX_CLOCK_SCALE)
        .max(1);

    (1..=max_scale)
        .rev()
        .find(|scale| {
            let block_height = DIGIT_HEIGHT * scale
                + schedule_block_height(schedule_line_count, *scale, schedule_text_scale(*scale));
            base_width * scale <= available_width && block_height <= available_height
        })
        .unwrap_or(1)
}

fn schedule_block_height(line_count: usize, clock_scale: usize, text_scale: usize) -> usize {
    if line_count == 0 {
        0
    } else {
        schedule_gap(clock_scale) + line_count * text_scale
    }
}

fn schedule_gap(scale: usize) -> usize {
    scale.clamp(1, 3) + 1
}

fn schedule_text_scale(clock_scale: usize) -> usize {
    if !is_kitty_terminal() {
        return 1;
    }

    clock_scale.div_ceil(2).clamp(1, 4)
}

fn usable_content_height(terminal_height: usize, alert: &AlertState) -> usize {
    let alert_reserved = if alert.is_active() { 3 } else { 1 };
    terminal_height.saturating_sub(alert_reserved)
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

#[derive(Clone)]
struct CalendarEvent {
    start: DateTime<Local>,
    end: Option<DateTime<Local>>,
    title: String,
    room: Option<String>,
}

struct CalendarState {
    events: Vec<CalendarEvent>,
    last_refresh: Option<Instant>,
    status: CalendarStatus,
}

enum CalendarStatus {
    Ready,
    Disabled,
    Error(String),
}

impl CalendarState {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            last_refresh: None,
            status: CalendarStatus::Disabled,
        }
    }

    fn refresh_if_due(&mut self, now: DateTime<Local>) {
        if self
            .last_refresh
            .is_some_and(|last_refresh| last_refresh.elapsed() < CALENDAR_REFRESH_INTERVAL)
        {
            return;
        }

        self.last_refresh = Some(Instant::now());
        match fetch_calendar_events(now) {
            Ok(events) => {
                self.events = events;
                self.status = CalendarStatus::Ready;
            }
            Err(error) => {
                self.events.clear();
                self.status = error;
            }
        }
    }

    fn current_event(&self, now: DateTime<Local>) -> Option<&CalendarEvent> {
        self.events.iter().find(|event| {
            event.start <= now && event.end.is_some_and(|end| end > now && end >= event.start)
        })
    }

    fn upcoming_event(&self, now: DateTime<Local>) -> Option<&CalendarEvent> {
        self.events.iter().find(|event| event.start > now)
    }

    fn alert_candidate(&self, now: DateTime<Local>) -> Option<AlertCandidate> {
        self.events
            .iter()
            .filter_map(|event| {
                let seconds_until = (event.start - now).num_seconds();
                if seconds_until <= ALERT_BEFORE_START.as_secs() as i64
                    && seconds_until > -(ALERT_GRACE_PERIOD.as_secs() as i64)
                {
                    Some(AlertCandidate {
                        event_id: event.id(),
                        title: event.display_title(),
                        kind: if seconds_until <= 0 {
                            AlertKind::Starting
                        } else {
                            AlertKind::TenMinutes
                        },
                    })
                } else {
                    None
                }
            })
            .next()
    }

    fn render_lines(&self, now: DateTime<Local>, width: usize) -> Vec<String> {
        if width < 12 {
            return Vec::new();
        }

        let mut lines = Vec::new();
        if let Some(current) = self.current_event(now) {
            lines.push(format!(
                "now  {}  {}",
                current.time_range(),
                current.display_title()
            ));
        }
        if let Some(upcoming) = self.upcoming_event(now) {
            lines.push(format!(
                "next {}  {}",
                upcoming.start.format("%H:%M"),
                upcoming.display_title()
            ));
        }

        if lines.is_empty() {
            match &self.status {
                CalendarStatus::Ready => lines.push("no upcoming calendar events".to_string()),
                CalendarStatus::Disabled => lines
                    .push("calendar: configure gws, gcalcli, or TIMECAT_CALENDAR_CMD".to_string()),
                CalendarStatus::Error(error) => lines.push(format!("calendar: {error}")),
            }
        }

        lines
            .into_iter()
            .take(2)
            .map(|line| truncate_to_width(&line, width))
            .collect()
    }

    fn render_key(&self, now: DateTime<Local>) -> String {
        self.render_lines(now, 200).join("\n")
    }
}

impl CalendarEvent {
    fn id(&self) -> String {
        format!("{}:{}", self.start.timestamp(), self.title)
    }

    fn display_title(&self) -> String {
        match &self.room {
            Some(room) if !room.is_empty() => format!("{} ({room})", self.title),
            _ => self.title.clone(),
        }
    }

    fn time_range(&self) -> String {
        match self.end {
            Some(end) => format!("{}-{}", self.start.format("%H:%M"), end.format("%H:%M")),
            None => self.start.format("%H:%M").to_string(),
        }
    }
}

fn fetch_calendar_events(now: DateTime<Local>) -> Result<Vec<CalendarEvent>, CalendarStatus> {
    if let Some(command) = std::env::var("TIMECAT_CALENDAR_CMD")
        .ok()
        .filter(|cmd| !cmd.is_empty())
    {
        return fetch_calendar_events_from_command(&command, now);
    }

    match fetch_calendar_events_from_gws(now) {
        Ok(events) => return Ok(events),
        Err(CalendarStatus::Disabled) => {}
        Err(error) => return Err(error),
    }

    fetch_calendar_events_from_gcalcli(now)
}

fn fetch_calendar_events_from_command(
    command: &str,
    now: DateTime<Local>,
) -> Result<Vec<CalendarEvent>, CalendarStatus> {
    let output =
        run_shell_command(command).map_err(|error| CalendarStatus::Error(error.to_string()))?;
    parse_tsv_calendar_output(&output, now)
}

fn fetch_calendar_events_from_gcalcli(
    now: DateTime<Local>,
) -> Result<Vec<CalendarEvent>, CalendarStatus> {
    let output = match Command::new("gcalcli")
        .args(["--nocolor", "agenda", "now", "tomorrow", "--tsv"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CalendarStatus::Disabled);
        }
        Err(error) => return Err(CalendarStatus::Error(error.to_string())),
    };

    parse_tsv_calendar_output(&output, now)
}

fn parse_tsv_calendar_output(
    output: &std::process::Output,
    now: DateTime<Local>,
) -> Result<Vec<CalendarEvent>, CalendarStatus> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.lines().next().unwrap_or("calendar command failed");
        return Err(CalendarStatus::Error(message.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut events: Vec<_> = stdout
        .lines()
        .filter_map(parse_calendar_event)
        .filter(|event| event.end.unwrap_or(event.start) >= now)
        .collect();
    events.sort_by_key(|event| event.start);
    Ok(events)
}

fn fetch_calendar_events_from_gws(
    now: DateTime<Local>,
) -> Result<Vec<CalendarEvent>, CalendarStatus> {
    let calendar_id = std::env::var("TIMECAT_GWS_CALENDAR_ID")
        .or_else(|_| std::env::var("TIMECAT_GOOGLE_CALENDAR_ID"))
        .unwrap_or_else(|_| "primary".to_string());
    let time_max = now + ChronoDuration::days(1);
    let params = serde_json::json!({
        "calendarId": calendar_id,
        "timeMin": now.to_rfc3339(),
        "timeMax": time_max.to_rfc3339(),
        "singleEvents": true,
        "orderBy": "startTime",
        "maxResults": 20,
    });

    let output = match Command::new("gws")
        .args(["calendar", "events", "list", "--params"])
        .arg(params.to_string())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CalendarStatus::Disabled);
        }
        Err(error) => return Err(CalendarStatus::Error(error.to_string())),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .next()
            .unwrap_or("gws calendar command failed");
        return Err(CalendarStatus::Error(message.to_string()));
    }

    let body: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| CalendarStatus::Error(error.to_string()))?;

    let mut events: Vec<_> = body
        .get("items")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_calendar_api_event)
        .filter(|event| event.end.unwrap_or(event.start) >= now)
        .collect();
    events.sort_by_key(|event| event.start);
    Ok(events)
}

fn parse_calendar_api_event(value: &serde_json::Value) -> Option<CalendarEvent> {
    let title = value
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(untitled)")
        .trim();
    let start = parse_calendar_api_event_time(value.get("start")?)?;
    let end = value.get("end").and_then(parse_calendar_api_event_time);
    let room = parse_calendar_api_room(value);

    Some(CalendarEvent {
        start,
        end,
        title: title.to_string(),
        room,
    })
}

fn parse_calendar_api_room(value: &serde_json::Value) -> Option<String> {
    value
        .get("attendees")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|attendee| {
            attendee
                .get("resource")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .and_then(|attendee| {
            attendee
                .get("displayName")
                .or_else(|| attendee.get("email"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| value.get("location").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .map(str::to_string)
}

fn parse_calendar_api_event_time(value: &serde_json::Value) -> Option<DateTime<Local>> {
    if let Some(datetime) = value.get("dateTime").and_then(serde_json::Value::as_str) {
        return parse_local_datetime(datetime);
    }

    let date = value.get("date").and_then(serde_json::Value::as_str)?;
    parse_local_datetime(&format!("{date} 00:00"))
}

fn run_shell_command(command: &str) -> io::Result<std::process::Output> {
    if cfg!(windows) {
        Command::new("cmd").args(["/C", command]).output()
    } else {
        Command::new("sh").args(["-c", command]).output()
    }
}

fn parse_calendar_event(line: &str) -> Option<CalendarEvent> {
    let fields: Vec<_> = line.split('\t').map(str::trim).collect();
    if fields.len() >= 5 {
        return parse_calendar_event_parts(
            &format!("{} {}", fields[0], fields[1]),
            Some(&format!("{} {}", fields[2], fields[3])),
            &fields[4..].join(" "),
        );
    }
    if fields.len() >= 3 {
        return parse_calendar_event_parts(fields[0], Some(fields[1]), &fields[2..].join(" "));
    }
    if fields.len() >= 2 {
        return parse_calendar_event_parts(fields[0], None, fields[1]);
    }

    None
}

fn parse_calendar_event_parts(
    start: &str,
    end: Option<&str>,
    title: &str,
) -> Option<CalendarEvent> {
    let start = parse_local_datetime(start)?;
    let end = end.and_then(parse_local_datetime);
    let title = title.trim();
    if title.is_empty() {
        return None;
    }

    Some(CalendarEvent {
        start,
        end,
        title: title.to_string(),
        room: None,
    })
}

fn parse_local_datetime(value: &str) -> Option<DateTime<Local>> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        return Some(datetime.with_timezone(&Local));
    }

    const FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
    ];

    FORMATS.iter().find_map(|format| {
        NaiveDateTime::parse_from_str(value, format)
            .ok()
            .and_then(|naive| Local.from_local_datetime(&naive).single())
    })
}

#[derive(Clone)]
struct AlertCandidate {
    event_id: String,
    title: String,
    kind: AlertKind,
}

#[derive(Clone, PartialEq, Eq)]
enum AlertKind {
    TenMinutes,
    Starting,
}

struct AlertState {
    active: Option<AlertCandidate>,
    dismissed_event_id: Option<String>,
    flash_started: Instant,
}

impl AlertState {
    fn new() -> Self {
        Self {
            active: None,
            dismissed_event_id: None,
            flash_started: Instant::now(),
        }
    }

    fn update(&mut self, _now: DateTime<Local>, candidate: Option<AlertCandidate>) {
        let Some(candidate) = candidate else {
            self.active = None;
            return;
        };

        if self
            .dismissed_event_id
            .as_ref()
            .is_some_and(|dismissed| dismissed == &candidate.event_id)
        {
            self.active = None;
            return;
        }

        if self.active.as_ref().is_none_or(|active| {
            active.event_id != candidate.event_id || active.kind != candidate.kind
        }) {
            self.flash_started = Instant::now();
        }
        self.active = Some(candidate);
    }

    fn dismiss(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        self.dismissed_event_id = Some(active.event_id);
        true
    }

    fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn foreground_color(&self) -> Color {
        if !self.is_active() {
            return Color::White;
        }

        match self.flash_phase() % 6 {
            0 => Color::Yellow,
            1 => Color::Cyan,
            2 => Color::Magenta,
            3 => Color::Green,
            4 => Color::Blue,
            _ => Color::Red,
        }
    }

    fn background_color(&self) -> Color {
        if !self.is_active() {
            return Color::Black;
        }

        match self.flash_phase() % 4 {
            0 => Color::Black,
            1 => Color::DarkBlue,
            2 => Color::DarkMagenta,
            _ => Color::DarkRed,
        }
    }

    fn message(&self) -> Option<String> {
        let active = self.active.as_ref()?;
        let prefix = match active.kind {
            AlertKind::TenMinutes => "10 min",
            AlertKind::Starting => "now",
        };
        Some(format!(
            "{prefix}: {}  (Esc/click to dismiss)",
            active.title
        ))
    }

    fn render_key(&self) -> String {
        if !self.is_active() {
            return String::new();
        }

        format!(
            "{}:{}",
            self.flash_phase(),
            self.message().unwrap_or_default()
        )
    }

    fn flash_phase(&self) -> u128 {
        self.flash_started.elapsed().as_millis() / ALERT_FLASH_INTERVAL.as_millis()
    }
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width < 3 {
        return String::new();
    }

    let mut truncated = String::new();
    let mut current_width = 0;
    let target_width = width - 3;
    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width + char_width > target_width {
            break;
        }
        truncated.push(ch);
        current_width += char_width;
    }
    truncated.push_str("...");
    truncated
}

fn centered_x(text: &str, terminal_width: u16) -> u16 {
    centered_x_scaled(text, terminal_width, 1)
}

fn centered_x_scaled(text: &str, terminal_width: u16, scale: usize) -> u16 {
    let width = display_width(text).saturating_mul(scale) as u16;
    terminal_width.saturating_sub(width) / 2
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn add_compact_time_fragment(
    frame: &mut [Vec<TextFragment>],
    time: &str,
    terminal_width: u16,
    terminal_height: u16,
) {
    if terminal_width == 0 || terminal_height == 0 {
        return;
    }

    let message: String = time.chars().take(terminal_width as usize).collect();
    let x = centered_x(&message, terminal_width);
    let y = terminal_height / 2;

    add_fragment(
        frame,
        y,
        x,
        message.clone(),
        display_width(&message),
        Color::White,
        None,
    );
}

fn add_kitty_compact_time_fragment(
    frame: &mut [Vec<TextFragment>],
    time: &str,
    terminal_width: u16,
    terminal_height: u16,
) {
    if terminal_width == 0 || terminal_height == 0 {
        return;
    }

    let message = time;
    let message_width = message.chars().count();
    if message_width == 0 {
        return;
    }
    if terminal_width as usize <= message_width {
        return add_compact_time_fragment(frame, time, terminal_width, terminal_height);
    }

    let scale = ((terminal_width as usize / message_width).min(terminal_height as usize))
        .clamp(1, MAX_KITTY_TEXT_SCALE);
    let content_width = message_width * scale;
    let x = terminal_width.saturating_sub(content_width as u16) / 2;
    let y = terminal_height.saturating_sub(scale as u16) / 2;

    add_fragment(
        frame,
        y,
        x,
        format!("\x1b]66;s={scale};{message}\x07"),
        content_width,
        Color::White,
        None,
    );
}

fn is_kitty_terminal() -> bool {
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.contains("xterm-kitty"))
}

enum InputAction {
    None,
    Dismissed,
    Quit,
}

fn handle_event(event: Event, alert: &mut AlertState) -> InputAction {
    match event {
        Event::Key(key) if key.code == KeyCode::Esc && alert.dismiss() => InputAction::Dismissed,
        Event::Key(key) if key.code == KeyCode::Esc => InputAction::Quit,
        Event::Key(key) if key.code == KeyCode::Char('q') => InputAction::Quit,
        Event::Key(key)
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            InputAction::Quit
        }
        Event::Mouse(_) if alert.dismiss() => InputAction::Dismissed,
        _ => InputAction::None,
    }
}

struct TerminalSession {
    stdout: io::Stdout,
}

impl TerminalSession {
    fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(
            self.stdout,
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen,
            ResetColor
        );
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_column_tsv_event() {
        let event =
            parse_calendar_event("2026-06-26 09:00\t2026-06-26 09:30\tStandup").expect("event");

        assert_eq!(
            event.start.format("%Y-%m-%d %H:%M").to_string(),
            "2026-06-26 09:00"
        );
        assert_eq!(
            event.end.expect("end").format("%Y-%m-%d %H:%M").to_string(),
            "2026-06-26 09:30"
        );
        assert_eq!(event.title, "Standup");
    }

    #[test]
    fn parses_five_column_tsv_event() {
        let event =
            parse_calendar_event("2026-06-26\t09:00\t2026-06-26\t09:30\tPlanning").expect("event");

        assert_eq!(
            event.start.format("%Y-%m-%d %H:%M").to_string(),
            "2026-06-26 09:00"
        );
        assert_eq!(event.title, "Planning");
    }

    #[test]
    fn parses_gws_calendar_event() {
        let value = serde_json::json!({
            "summary": "Review",
            "start": { "dateTime": "2026-06-26T10:00:00+09:00" },
            "end": { "dateTime": "2026-06-26T10:30:00+09:00" },
            "attendees": [
                {
                    "displayName": "Focus Room",
                    "email": "focus-room@example.com",
                    "resource": true
                }
            ]
        });
        let event = parse_calendar_api_event(&value).expect("event");

        assert_eq!(
            event.start.format("%Y-%m-%d %H:%M").to_string(),
            "2026-06-26 10:00"
        );
        assert_eq!(event.title, "Review");
        assert_eq!(event.room.as_deref(), Some("Focus Room"));
        assert_eq!(event.display_title(), "Review (Focus Room)");
    }

    #[test]
    fn centers_wide_schedule_text_by_terminal_width() {
        assert_eq!(display_width("일정"), 4);
        assert_eq!(centered_x("일정", 20), 8);
    }

    #[test]
    fn dismisses_active_alert() {
        let mut alert = AlertState::new();
        alert.update(
            Local::now(),
            Some(AlertCandidate {
                event_id: "event-1".to_string(),
                title: "Demo".to_string(),
                kind: AlertKind::TenMinutes,
            }),
        );

        assert!(alert.is_active());
        assert!(alert.dismiss());
        assert!(!alert.is_active());
    }
}
