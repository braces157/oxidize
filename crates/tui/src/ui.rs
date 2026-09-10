//! Ratatui UI drawing functions and multi-panel layout rendering.

use crate::app::App;
use crate::model::{ActiveModal, DiffLineKind, Panel};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

/// Renders the complete multi-panel TUI interface onto the terminal frame.
pub fn render(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // Vertical layout: Header (3), Main Body (Min 0), Footer (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(size);

    render_header(frame, app, chunks[0]);
    render_body(frame, app, chunks[1]);
    render_footer(frame, app, chunks[2]);
    render_modals(frame, app);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mut header_spans = vec![
        Span::styled(
            " 🦀 OXIDIZE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(220, 90, 40))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " [LazyOx] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ Branch: "),
        Span::styled(
            &app.branch_name,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ Active: "),
        Span::styled(
            app.active_panel.title(),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if let Some(ref msg) = app.status_message {
        header_spans.push(Span::raw(" │ "));
        header_spans.push(Span::styled(
            msg,
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(Line::from(header_spans)).block(block);
    frame.render_widget(paragraph, area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    // Horizontal layout: Left Dock (38%), Right Inspector (62%)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    render_left_dock(frame, app, h_chunks[0]);
    render_inspector(frame, app, h_chunks[1]);
}

fn render_left_dock(frame: &mut Frame, app: &App, area: Rect) {
    // Left dock vertical 4-panel split: Files (35%), Branches (20%), Commits (35%), Stash (10%)
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(20),
            Constraint::Percentage(35),
            Constraint::Percentage(10),
        ])
        .split(area);

    render_files_panel(frame, app, v_chunks[0]);
    render_branches_panel(frame, app, v_chunks[1]);
    render_commits_panel(frame, app, v_chunks[2]);
    render_stash_panel(frame, app, v_chunks[3]);
}

fn get_panel_border(app: &App, panel: Panel) -> Block<'static> {
    let is_focused = app.active_panel == panel;
    let (border_color, title_color) = if is_focused {
        (Color::Yellow, Color::Yellow)
    } else {
        (Color::DarkGray, Color::Gray)
    };

    let title = format!(" {} ", panel.title());
    Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            Style::default()
                .fg(title_color)
                .add_modifier(if is_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ))
}

fn render_files_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = get_panel_border(app, Panel::Files);
    let is_panel_focused = app.active_panel == Panel::Files;

    let items: Vec<ListItem> = if app.files.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (working tree clean)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.files
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let is_selected = idx == app.files_selected;
                let marker = if is_selected && is_panel_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let (badge, badge_color) = f.kind.badge();
                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("{} ", badge),
                        Style::default()
                            .fg(badge_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &f.path,
                        Style::default().fg(if is_selected {
                            Color::White
                        } else {
                            Color::Gray
                        }),
                    ),
                ]);

                let item = ListItem::new(line);
                if is_selected && is_panel_focused {
                    item.style(Style::default().bg(Color::Rgb(35, 45, 60)))
                } else {
                    item
                }
            })
            .collect()
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_branches_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = get_panel_border(app, Panel::Branches);
    let is_panel_focused = app.active_panel == Panel::Branches;

    let items: Vec<ListItem> = if app.branches.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no branches)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.branches
            .iter()
            .enumerate()
            .map(|(idx, b)| {
                let is_selected = idx == app.branches_selected;
                let marker = if is_selected && is_panel_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let head_indicator = if b.is_head { "* " } else { "  " };
                let branch_color = if b.is_head {
                    Color::Green
                } else if b.is_remote {
                    Color::LightRed
                } else if is_selected {
                    Color::White
                } else {
                    Color::Cyan
                };

                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        head_indicator,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &b.name,
                        Style::default()
                            .fg(branch_color)
                            .add_modifier(if b.is_head {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ]);

                let item = ListItem::new(line);
                if is_selected && is_panel_focused {
                    item.style(Style::default().bg(Color::Rgb(35, 45, 60)))
                } else {
                    item
                }
            })
            .collect()
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_commits_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = get_panel_border(app, Panel::Commits);
    let is_panel_focused = app.active_panel == Panel::Commits;

    let items: Vec<ListItem> = if app.commits.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no commits)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.commits
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let is_selected = idx == app.commits_selected;
                let marker = if is_selected && is_panel_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        &c.short_oid,
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        &c.summary,
                        Style::default().fg(if is_selected {
                            Color::White
                        } else {
                            Color::LightCyan
                        }),
                    ),
                ]);

                let item = ListItem::new(line);
                if is_selected && is_panel_focused {
                    item.style(Style::default().bg(Color::Rgb(35, 45, 60)))
                } else {
                    item
                }
            })
            .collect()
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_stash_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = get_panel_border(app, Panel::Stash);
    let is_panel_focused = app.active_panel == Panel::Stash;

    let items: Vec<ListItem> = if app.stashes.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (stash stack empty)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.stashes
            .iter()
            .enumerate()
            .map(|(idx, s)| {
                let is_selected = idx == app.stashes_selected;
                let marker = if is_selected && is_panel_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("stash@{{{}}} ", s.index),
                        Style::default()
                            .fg(Color::LightYellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &s.message,
                        Style::default().fg(if is_selected {
                            Color::White
                        } else {
                            Color::Gray
                        }),
                    ),
                ]);

                let item = ListItem::new(line);
                if is_selected && is_panel_focused {
                    item.style(Style::default().bg(Color::Rgb(35, 45, 60)))
                } else {
                    item
                }
            })
            .collect()
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_inspector(frame: &mut Frame, app: &App, area: Rect) {
    let title = app
        .cached_diff
        .as_ref()
        .map(|d| format!(" {} ", d.title))
        .unwrap_or_else(|| " Inspector ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightBlue))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let diff_view = match app.cached_diff {
        Some(ref d) => d,
        None => {
            let paragraph =
                Paragraph::new("No content").block(block);
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let lines: Vec<Line> = diff_view
        .lines
        .iter()
        .map(|dl| {
            let style = match dl.kind {
                DiffLineKind::Header => Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                DiffLineKind::HunkHeader => Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                DiffLineKind::Addition => Style::default().fg(Color::Green),
                DiffLineKind::Deletion => Style::default().fg(Color::Red),
                DiffLineKind::Context => Style::default().fg(Color::Gray),
                DiffLineKind::Normal => Style::default().fg(Color::White),
            };
            Line::from(Span::styled(&dl.content, style))
        })
        .collect();

    let total_lines = lines.len();
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.inspector_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    // Scroll indicator badge in bottom right corner if scrollable
    if total_lines > area.height as usize && area.height > 2 {
        let scroll_pct = (app.inspector_scroll * 100)
            .checked_div(total_lines.saturating_sub(area.height as usize))
            .unwrap_or(0)
            .min(100);
        let indicator = format!(" [{}%] ", scroll_pct);
        let ind_rect = Rect {
            x: area.x + area.width.saturating_sub(indicator.len() as u16 + 2),
            y: area.y + area.height.saturating_sub(1),
            width: indicator.len() as u16,
            height: 1,
        };
        let ind_p = Paragraph::new(Span::styled(
            indicator,
            Style::default().fg(Color::Yellow),
        ));
        frame.render_widget(ind_p, ind_rect);
    }
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let help_line = match app.active_panel {
        Panel::Files => Line::from(vec![
            Span::styled(
                " [Space] ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Stage/Unstage │ "),
            Span::styled(
                "c",
                Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Commit │ "),
            Span::styled(
                "a",
                Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Stage All │ "),
            Span::styled(
                "d",
                Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Discard │ "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" Help │ "),
            Span::styled("[1-4]", Style::default().fg(Color::Cyan)),
            Span::raw(" Panels │ "),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::raw(" Quit"),
        ]),
        Panel::Branches => Line::from(vec![
            Span::styled(
                " [Space/Enter] ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Checkout │ "),
            Span::styled(
                "n",
                Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" New Branch │ "),
            Span::styled(
                "d",
                Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Delete Branch │ "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" Help │ "),
            Span::styled("[1-4]", Style::default().fg(Color::Cyan)),
            Span::raw(" Panels │ "),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::raw(" Quit"),
        ]),
        Panel::Commits => Line::from(vec![
            Span::styled(
                " j/k ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Select Commit │ "),
            Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
            Span::raw(" Scroll Diff │ "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" Help │ "),
            Span::styled("[1-4]", Style::default().fg(Color::Cyan)),
            Span::raw(" Panels │ "),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::raw(" Quit"),
        ]),
        Panel::Stash => Line::from(vec![
            Span::styled(
                " [Space/Enter] ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Pop Stash │ "),
            Span::styled(
                "d",
                Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Drop Stash │ "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" Help │ "),
            Span::styled("[1-4]", Style::default().fg(Color::Cyan)),
            Span::raw(" Panels │ "),
            Span::styled("q", Style::default().fg(Color::Red)),
            Span::raw(" Quit"),
        ]),
    };

    let paragraph = Paragraph::new(help_line).style(Style::default().bg(Color::Rgb(20, 20, 25)));
    frame.render_widget(paragraph, area);
}

fn render_modals(frame: &mut Frame, app: &App) {
    match app.active_modal {
        ActiveModal::None => {}
        ActiveModal::CommitPrompt { ref message, cursor } => {
            let area = centered_rect(65, 30, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::LightGreen))
                .title(Span::styled(
                    " 💬 Commit Staged Changes (Enter: Submit, Esc: Cancel) ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if message.is_empty() {
                Span::styled(
                    "Enter commit summary (e.g. feat: add new feature)...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(message, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
                .split(inner);

            let prompt_title = Span::styled(
                "Commit message:",
                Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD),
            );
            frame.render_widget(Paragraph::new(prompt_title), v_chunks[0]);
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            let hint = Span::styled(
                "Enter to create commit object and advance HEAD",
                Style::default().fg(Color::DarkGray),
            );
            frame.render_widget(Paragraph::new(hint), v_chunks[2]);

            // Set cursor position on screen
            frame.set_cursor_position((
                v_chunks[1].x + cursor as u16,
                v_chunks[1].y,
            ));
        }
        ActiveModal::BranchCreate { ref name, cursor } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " 🌱 Create & Checkout New Branch (Enter: Create, Esc: Cancel) ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if name.is_empty() {
                Span::styled("Enter new branch name...", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(name, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);

            frame.render_widget(Paragraph::new(Span::styled("New Branch Name:", Style::default().fg(Color::LightGreen))), v_chunks[0]);
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            frame.set_cursor_position((
                v_chunks[1].x + cursor as u16,
                v_chunks[1].y,
            ));
        }
        ActiveModal::Help => {
            let area = centered_rect(70, 75, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    " 📖 Keyboard Shortcuts Cheatsheet (Esc: Close) ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled("Docked Panel Navigation:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from("  1, 2, 3, 4      Jump directly to panel (Files, Branches, Commits, Stash)"),
                Line::from("  Tab / Shift+Tab Cycle forward / backward between panels"),
                Line::from("  j / k (Down/Up) Navigate items within the focused panel"),
                Line::from("  PgUp / PgDn     Scroll Inspector diff pane up / down"),
                Line::from("  Ctrl+u / Ctrl+d Fast half-page scroll in Inspector"),
                Line::from(""),
                Line::from(Span::styled("Files Panel Actions:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
                Line::from("  Space           Toggle stage / unstage for selected file"),
                Line::from("  c               Open commit modal dialog for staged changes"),
                Line::from("  a               Stage all changes / Unstage all if already staged"),
                Line::from("  d               Discard unstaged modifications or untracked file"),
                Line::from(""),
                Line::from(Span::styled("Branches Panel Actions:", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD))),
                Line::from("  Space / Enter   Switch / checkout selected branch"),
                Line::from("  n               Create and checkout new branch"),
                Line::from("  d               Delete selected branch (with confirmation)"),
                Line::from(""),
                Line::from(Span::styled("Stash Panel Actions:", Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD))),
                Line::from("  Space           Pop selected stash into working tree"),
                Line::from("  d               Drop selected stash entry"),
                Line::from(""),
                Line::from(Span::styled("General:", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))),
                Line::from("  r               Refresh repository status in place"),
                Line::from("  ?               Toggle this help cheatsheet"),
                Line::from("  q / Esc         Exit LazyOx / close active popup"),
            ];

            let p = Paragraph::new(lines).block(block);
            frame.render_widget(p, area);
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
