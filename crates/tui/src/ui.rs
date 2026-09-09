//! Ratatui UI drawing functions and layout rendering.

use crate::app::{App, TabMode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

/// Renders the full TUI interface onto the terminal frame.
pub fn render(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // Vertical layout: Header (3), Main Body (Min 0), Footer (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(size);

    render_header(frame, app, chunks[0]);
    render_body(frame, app, chunks[1]);
    render_footer(frame, chunks[2]);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode_str = match app.active_tab {
        TabMode::Commits => "LOG GRAPH",
        TabMode::Status => "STATUS & DIFF",
    };

    let title = Line::from(vec![
        Span::styled(
            " 🦀 OXIDIZE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(220, 90, 40))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Branch: "),
        Span::styled(
            &app.branch_name,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  Mode: "),
        Span::styled(
            mode_str,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(title).block(block);
    frame.render_widget(paragraph, area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    // Horizontal layout: Left panel (45%), Right panel (55%)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_commit_list(frame, app, h_chunks[0]);
    render_details_panel(frame, app, h_chunks[1]);
}

fn render_commit_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .commits
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let is_selected = idx == app.selected_index;
            let marker = if is_selected { "▶ " } else { "  " };

            let line = Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Yellow)),
                Span::styled(
                    &c.short_oid,
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(&c.summary, Style::default().fg(Color::White)),
                Span::styled(
                    format!(" ({})", c.author),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            let style = if is_selected {
                Style::default()
                    .bg(Color::Rgb(40, 44, 52))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Commits ")
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_details_panel(frame: &mut Frame, app: &App, area: Rect) {
    match app.active_tab {
        TabMode::Commits => {
            // Split into Top: Commit Details, Bottom: Status Overview
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(area);

            if let Some(c) = app.selected_commit() {
                let lines = vec![
                    Line::from(vec![
                        Span::styled("commit: ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            c.oid.to_string(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("author: ", Style::default().fg(Color::Yellow)),
                        Span::raw(&c.author),
                    ]),
                    Line::from(vec![
                        Span::styled("parents: ", Style::default().fg(Color::Yellow)),
                        Span::raw(
                            c.parents
                                .iter()
                                .map(|p| p.to_string()[..7].to_string())
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ]),
                    Line::raw(""),
                    Line::styled(
                        &c.full_message,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ];

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Commit Details ")
                    .border_style(Style::default().fg(Color::Green));

                let p = Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false });
                frame.render_widget(p, v_chunks[0]);
            } else {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Commit Details ");
                frame.render_widget(Paragraph::new("No commits found").block(block), v_chunks[0]);
            }

            render_status_box(frame, app, v_chunks[1]);
        }
        TabMode::Status => {
            render_status_box(frame, app, area);
        }
    }
}

fn render_status_box(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .status_lines
        .iter()
        .map(|s| {
            let style = if s.contains("committed:") {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else if s.contains("not staged:") {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if s.contains("Untracked") {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            Line::styled(s, style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Working Tree Status ")
        .border_style(Style::default().fg(Color::Blue));

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer_text = Line::from(vec![
        Span::styled(
            " [q/Esc] ",
            Style::default().fg(Color::Black).bg(Color::White),
        ),
        Span::raw(" Quit  "),
        Span::styled(
            " [j/↓] ",
            Style::default().fg(Color::Black).bg(Color::White),
        ),
        Span::raw(" Down  "),
        Span::styled(
            " [k/↑] ",
            Style::default().fg(Color::Black).bg(Color::White),
        ),
        Span::raw(" Up  "),
        Span::styled(
            " [Tab] ",
            Style::default().fg(Color::Black).bg(Color::White),
        ),
        Span::raw(" Switch Mode  "),
    ]);

    let paragraph = Paragraph::new(footer_text);
    frame.render_widget(paragraph, area);
}
