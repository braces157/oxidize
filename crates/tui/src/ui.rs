//! Ratatui UI drawing functions and multi-panel layout rendering (LazyOx - authentic LazyGit replica).

use crate::app::App;
use crate::model::{ActiveModal, BranchesTab, CommitsTab, DiffLineKind, FocusedWindow, Panel};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

/// Computed bounding geometry for all panels and views in the LazyGit dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppLayout {
    pub screen: Rect,
    pub workspace: Rect,
    pub left_dock: Rect,
    pub status_panel: Rect,
    pub files_panel: Rect,
    pub branches_panel: Rect,
    pub commits_panel: Rect,
    pub stash_panel: Rect,
    pub inspector: Rect,
    pub footer: Rect,
}

/// Computes the complete deterministic layout rectangles for the current screen size.
pub fn compute_layout(size: Rect) -> Option<AppLayout> {
    if size.width < 20 || size.height < 6 {
        return None;
    }

    // Vertical layout: Main Workspace (Min 6), Bottom Status/Feedback Bar (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(1)])
        .split(size);

    let workspace = chunks[0];
    let footer = chunks[1];

    // Horizontal layout: Left Dock (35%), Right Inspector (65%)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(workspace);

    let left_dock = h_chunks[0];
    let inspector = h_chunks[1];

    // Left dock 5-panel split: Status (5 lines), Files (35%), Branches (24%), Commits (26%), Stash (Min 4)
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Percentage(35),
            Constraint::Percentage(24),
            Constraint::Percentage(26),
            Constraint::Min(4),
        ])
        .split(left_dock);

    Some(AppLayout {
        screen: size,
        workspace,
        left_dock,
        status_panel: v_chunks[0],
        files_panel: v_chunks[1],
        branches_panel: v_chunks[2],
        commits_panel: v_chunks[3],
        stash_panel: v_chunks[4],
        inspector,
        footer,
    })
}

/// Renders the complete authentic LazyGit interface onto the terminal frame.
pub fn render(frame: &mut Frame, app: &App) {
    let size = frame.area();

    let layout = match compute_layout(size) {
        Some(l) => l,
        None => {
            let msg = Paragraph::new("Terminal window too small")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(msg, size);
            return;
        }
    };

    render_status_panel(frame, app, layout.status_panel);
    render_files_panel(frame, app, layout.files_panel);
    render_branches_panel(frame, app, layout.branches_panel);
    render_commits_panel(frame, app, layout.commits_panel);
    render_stash_panel(frame, app, layout.stash_panel);
    render_inspector(frame, app, layout.inspector);
    render_footer(frame, app, layout.footer);
    render_modals(frame, app);
}

fn is_panel_active(app: &App, panel: Panel) -> bool {
    app.active_panel == panel && app.focused_window == FocusedWindow::Sidebar
}

fn get_panel_border(app: &App, panel: Panel, title_line: Line<'static>) -> Block<'static> {
    let is_focused = is_panel_active(app, panel);
    let border_color = if is_focused {
        Color::Green
    } else {
        Color::DarkGray
    };

    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title_line)
}

fn render_status_panel(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = is_panel_active(app, Panel::Status);
    let title_line = Line::from(vec![Span::styled(
        " 1 Status ",
        Style::default()
            .fg(if is_focused {
                Color::Green
            } else {
                Color::White
            })
            .add_modifier(if is_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )]);

    let block = get_panel_border(app, Panel::Status, title_line);

    let repo_name = app
        .repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Repository".to_string());

    let staged_count = app.files.iter().filter(|f| f.kind.is_staged()).count();
    let unstaged_count = app.files.iter().filter(|f| !f.kind.is_staged()).count();
    let status_desc = if staged_count + unstaged_count == 0 {
        Span::styled("Clean", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            format!("{} staged, {} unstaged", staged_count, unstaged_count),
            Style::default().fg(Color::Yellow),
        )
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Repo:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                repo_name,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Branch: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("* {}", app.branch_name),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("(↑{} ↓{})", app.ahead_behind.0, app.ahead_behind.1),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
            status_desc,
        ]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

pub fn window_range(total: usize, selected: usize, height: usize) -> (usize, usize) {
    if total == 0 || height == 0 {
        return (0, 0);
    }
    if total <= height {
        return (0, total);
    }
    let start = if selected < height / 2 {
        0
    } else if selected + height / 2 >= total {
        total.saturating_sub(height)
    } else {
        selected - height / 2
    };
    let end = (start + height).min(total);
    (start, end)
}

fn render_files_panel(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = is_panel_active(app, Panel::Files);
    let title_line = Line::from(vec![Span::styled(
        " 2 Files ",
        Style::default()
            .fg(if is_focused {
                Color::Green
            } else {
                Color::White
            })
            .add_modifier(if is_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )]);

    let block = get_panel_border(app, Panel::Files, title_line);
    let avail_height = (area.height as usize).saturating_sub(2);
    let (start_idx, end_idx) = window_range(app.files.len(), app.files_selected, avail_height);

    let items: Vec<ListItem> = if app.files.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (working tree clean)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.files[start_idx..end_idx]
            .iter()
            .enumerate()
            .map(|(offset, f)| {
                let actual_idx = start_idx + offset;
                let is_selected = actual_idx == app.files_selected;
                let marker = if is_selected && is_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let (badge, badge_color) = f.kind.badge();
                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Green)),
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
                if is_selected && is_focused {
                    item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
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
    let is_focused = is_panel_active(app, Panel::Branches);

    let mut tab_spans = vec![Span::styled(
        " 3 Branches ",
        Style::default()
            .fg(if is_focused {
                Color::Green
            } else {
                Color::White
            })
            .add_modifier(if is_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )];

    let tabs = [
        (BranchesTab::Local, "Local"),
        (BranchesTab::Remotes, "Remotes"),
        (BranchesTab::Tags, "Tags"),
    ];

    for (t, label) in tabs {
        if app.branches_tab == t {
            tab_spans.push(Span::styled(
                format!("[ {} ]", label),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(30, 60, 45))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::DarkGray),
            ));
        }
        tab_spans.push(Span::raw(" "));
    }

    let block = get_panel_border(app, Panel::Branches, Line::from(tab_spans));

    let avail_height = (area.height as usize).saturating_sub(2);

    let items: Vec<ListItem> = match app.branches_tab {
        BranchesTab::Local => {
            let local_branches: Vec<&crate::model::BranchItem> =
                app.branches.iter().filter(|b| !b.is_remote).collect();
            let (start_idx, end_idx) =
                window_range(local_branches.len(), app.branches_selected, avail_height);

            if local_branches.is_empty() {
                vec![ListItem::new(Span::styled(
                    "  (no local branches)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                local_branches[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, b)| {
                        let actual_idx = start_idx + offset;
                        let is_selected = actual_idx == app.branches_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        let head_indicator = if b.is_head { "* " } else { "  " };
                        let branch_color = if b.is_head {
                            Color::Green
                        } else if is_selected {
                            Color::White
                        } else {
                            Color::Cyan
                        };

                        let mut line_spans = vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
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
                        ];

                        if b.is_head && (app.ahead_behind.0 > 0 || app.ahead_behind.1 > 0) {
                            line_spans.push(Span::raw(" "));
                            line_spans.push(Span::styled(
                                format!("(↑{} ↓{})", app.ahead_behind.0, app.ahead_behind.1),
                                Style::default().fg(Color::Yellow),
                            ));
                        }

                        let item = ListItem::new(Line::from(line_spans));
                        if is_selected && is_focused {
                            item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
                        } else {
                            item
                        }
                    })
                    .collect()
            }
        }
        BranchesTab::Remotes => {
            let (start_idx, end_idx) =
                window_range(app.remotes.len(), app.remotes_selected, avail_height);

            if app.remotes.is_empty() {
                vec![ListItem::new(Span::styled(
                    "  (no remotes configured)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                app.remotes[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, r)| {
                        let actual_idx = start_idx + offset;
                        let is_selected = actual_idx == app.remotes_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                &r.name,
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(&r.url, Style::default().fg(Color::DarkGray)),
                        ]);

                        let item = ListItem::new(line);
                        if is_selected && is_focused {
                            item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
                        } else {
                            item
                        }
                    })
                    .collect()
            }
        }
        BranchesTab::Tags => {
            let (start_idx, end_idx) =
                window_range(app.tags.len(), app.tags_selected, avail_height);

            if app.tags.is_empty() {
                vec![ListItem::new(Span::styled(
                    "  (no tags)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                app.tags[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, t)| {
                        let actual_idx = start_idx + offset;
                        let is_selected = actual_idx == app.tags_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                &t.name,
                                Style::default()
                                    .fg(Color::Magenta)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("({})", t.short_oid),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]);

                        let item = ListItem::new(line);
                        if is_selected && is_focused {
                            item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
                        } else {
                            item
                        }
                    })
                    .collect()
            }
        }
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_commits_panel(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = is_panel_active(app, Panel::Commits);

    let mut tab_spans = vec![Span::styled(
        " 4 Commits ",
        Style::default()
            .fg(if is_focused {
                Color::Green
            } else {
                Color::White
            })
            .add_modifier(if is_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )];

    let tabs = [
        (CommitsTab::Commits, "Commits"),
        (CommitsTab::Reflog, "Reflog"),
    ];

    for (t, label) in tabs {
        if app.commits_tab == t {
            tab_spans.push(Span::styled(
                format!("[ {} ]", label),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(30, 60, 45))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::DarkGray),
            ));
        }
        tab_spans.push(Span::raw(" "));
    }

    let block = get_panel_border(app, Panel::Commits, Line::from(tab_spans));

    let avail_height = (area.height as usize).saturating_sub(2);

    let items: Vec<ListItem> = match app.commits_tab {
        CommitsTab::Commits => {
            let (start_idx, end_idx) =
                window_range(app.commits.len(), app.commits_selected, avail_height);

            if app.commits.is_empty() {
                vec![ListItem::new(Span::styled(
                    "  (no commits in repository)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                app.commits[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, c)| {
                        let actual_idx = start_idx + offset;
                        let is_selected = actual_idx == app.commits_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        // Authentic LazyGit commit graph node
                        let graph_node = "* ";

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                graph_node,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                &c.short_oid,
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                &c.summary,
                                Style::default().fg(if is_selected {
                                    Color::White
                                } else {
                                    Color::Gray
                                }),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("({})", c.author),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]);

                        let item = ListItem::new(line);
                        if is_selected && is_focused {
                            item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
                        } else {
                            item
                        }
                    })
                    .collect()
            }
        }
        CommitsTab::Reflog => {
            let (start_idx, end_idx) =
                window_range(app.reflog.len(), app.reflog_selected, avail_height);

            if app.reflog.is_empty() {
                vec![ListItem::new(Span::styled(
                    "  (reflog is empty)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                app.reflog[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, entry)| {
                        let actual_idx = start_idx + offset;
                        let is_selected = actual_idx == app.reflog_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                &entry.selector,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("{}: ", entry.action),
                                Style::default().fg(Color::Yellow),
                            ),
                            Span::styled(
                                &entry.message,
                                Style::default().fg(if is_selected {
                                    Color::White
                                } else {
                                    Color::Gray
                                }),
                            ),
                        ]);

                        let item = ListItem::new(line);
                        if is_selected && is_focused {
                            item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
                        } else {
                            item
                        }
                    })
                    .collect()
            }
        }
    };

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_stash_panel(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = is_panel_active(app, Panel::Stash);
    let title_line = Line::from(vec![Span::styled(
        " 5 Stash ",
        Style::default()
            .fg(if is_focused {
                Color::Green
            } else {
                Color::White
            })
            .add_modifier(if is_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )]);

    let block = get_panel_border(app, Panel::Stash, title_line);
    let avail_height = (area.height as usize).saturating_sub(2);
    let (start_idx, end_idx) = window_range(app.stashes.len(), app.stashes_selected, avail_height);

    let items: Vec<ListItem> = if app.stashes.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (stash stack empty)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.stashes[start_idx..end_idx]
            .iter()
            .enumerate()
            .map(|(offset, s)| {
                let actual_idx = start_idx + offset;
                let is_selected = actual_idx == app.stashes_selected;
                let marker = if is_selected && is_focused {
                    "▶ "
                } else if is_selected {
                    "▷ "
                } else {
                    "  "
                };

                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Green)),
                    Span::styled(
                        format!("stash@{{{}}}: ", s.index),
                        Style::default()
                            .fg(Color::Yellow)
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
                if is_selected && is_focused {
                    item.style(Style::default().bg(Color::Rgb(25, 45, 35)))
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
    let is_inspector_focused = app.focused_window == FocusedWindow::Inspector;
    let border_color = if is_inspector_focused {
        Color::Green
    } else {
        Color::DarkGray
    };

    let base_title = app
        .cached_diff
        .as_ref()
        .map(|d| d.title.clone())
        .unwrap_or_else(|| "Main".to_string());

    let title_text = if is_inspector_focused {
        format!(
            " [ {} ] ── [ FOCUSED: j/k (or Mouse) Scroll │ PgDn/PgUp Page │ Esc/h Return ] ",
            base_title
        )
    } else {
        format!(
            " [ {} ] ── [ Enter/l to Focus & Scroll │ Mouse/PgDn to Scroll ] ",
            base_title
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title_text,
            Style::default()
                .fg(if is_inspector_focused {
                    Color::Green
                } else {
                    Color::Cyan
                })
                .add_modifier(Modifier::BOLD),
        ));

    let diff_view = match app.cached_diff {
        Some(ref d) => d,
        None => {
            let paragraph = Paragraph::new("No content").block(block);
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
                DiffLineKind::Deletion => Style::default().fg(Color::LightRed),
                DiffLineKind::Context => Style::default().fg(Color::DarkGray),
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
    if total_lines > 0 && area.height > 2 {
        let max_scroll = total_lines.saturating_sub(area.height as usize);
        let current_line = (app.inspector_scroll + 1).min(total_lines);
        let scroll_pct = if max_scroll > 0 {
            (app.inspector_scroll * 100)
                .checked_div(max_scroll)
                .unwrap_or(0)
                .min(100)
        } else {
            100
        };
        let indicator = format!(
            " [Line {}/{} - {}%] ",
            current_line, total_lines, scroll_pct
        );
        let ind_rect = Rect {
            x: area.x + area.width.saturating_sub(indicator.len() as u16 + 2),
            y: area.y + area.height.saturating_sub(1),
            width: indicator.len() as u16,
            height: 1,
        };
        let ind_p = Paragraph::new(Span::styled(
            indicator,
            Style::default().fg(if is_inspector_focused {
                Color::Green
            } else {
                Color::Yellow
            }),
        ));
        frame.render_widget(ind_p, ind_rect);
    }
}

/// Clickable action categories available in the bottom footer status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterAction {
    Quit,
    Help,
    Push,
    Pull,
    Refresh,
    FocusToggle,
    CyclePanel,
    StageToggle,
    StageAll,
    Commit,
    Amend,
    StashSave,
    Discard,
    ScrollDiff,
    CheckoutBranch,
    NewBranch,
    DeleteBranch,
    SwitchTab,
    PopStash,
    ApplyStash,
    DropStash,
    JumpStatus,
    JumpFiles,
    JumpBranches,
    JumpCommits,
    JumpStash,
    ReturnToSidebar,
    ScrollInspectorTop,
    ScrollInspectorBottom,
    PageInspectorDown,
}

/// Builds footer text spans and computes clickable bounding columns for footer actions.
pub fn build_footer(app: &App, _width: u16) -> (Line<'static>, Vec<(u16, u16, FooterAction)>) {
    let mut footer_spans = Vec::new();
    let mut buttons = Vec::new();
    let mut col_offset: u16 = 0;

    // Feedback message or default badge
    if let Some(ref msg) = app.status_message {
        let text = format!(" {} ", msg);
        col_offset += text.len() as u16;
        footer_spans.push(Span::styled(
            text,
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(40, 30, 20))
                .add_modifier(Modifier::BOLD),
        ));
        footer_spans.push(Span::raw(" │ "));
        col_offset += 3;
    } else {
        let text = " [LazyOx] Ready ";
        col_offset += text.len() as u16;
        footer_spans.push(Span::styled(
            text,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        footer_spans.push(Span::raw(" │ "));
        col_offset += 3;
    }

    let push_btn = |key: &'static str,
                    key_style: Style,
                    label: &'static str,
                    label_style: Style,
                    action: FooterAction,
                    spans: &mut Vec<Span<'static>>,
                    btns: &mut Vec<(u16, u16, FooterAction)>,
                    offset: &mut u16| {
        let start_x = *offset;
        spans.push(Span::styled(key, key_style));
        spans.push(Span::styled(label, label_style));
        let btn_len = (key.len() + label.len()) as u16;
        *offset += btn_len;
        btns.push((start_x, *offset, action));
        spans.push(Span::raw(" │ "));
        *offset += 3;
    };

    let bold_green = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let bold_cyan = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let bold_yellow = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let bold_red = Style::default()
        .fg(Color::LightRed)
        .add_modifier(Modifier::BOLD);
    let cyan = Style::default().fg(Color::Cyan);
    let green = Style::default().fg(Color::Green);
    let yellow = Style::default().fg(Color::Yellow);
    let raw = Style::default();

    if app.focused_window == FocusedWindow::Inspector {
        push_btn(
            "j/k",
            bold_green,
            " (↑/↓) Scroll",
            raw,
            FooterAction::ScrollDiff,
            &mut footer_spans,
            &mut buttons,
            &mut col_offset,
        );
        push_btn(
            "PgDn/PgUp",
            cyan,
            " Page",
            raw,
            FooterAction::PageInspectorDown,
            &mut footer_spans,
            &mut buttons,
            &mut col_offset,
        );
        push_btn(
            "g/G",
            yellow,
            " Top/Bottom",
            raw,
            FooterAction::ScrollInspectorTop,
            &mut footer_spans,
            &mut buttons,
            &mut col_offset,
        );
        push_btn(
            "Esc/h",
            bold_red,
            " Return to Sidebar",
            raw,
            FooterAction::ReturnToSidebar,
            &mut footer_spans,
            &mut buttons,
            &mut col_offset,
        );
    } else {
        match app.active_panel {
            Panel::Status => {
                push_btn(
                    "1-5",
                    bold_cyan,
                    " Jump",
                    raw,
                    FooterAction::CyclePanel,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "Tab",
                    cyan,
                    " Cycle",
                    raw,
                    FooterAction::CyclePanel,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "h/l",
                    cyan,
                    " Window",
                    raw,
                    FooterAction::FocusToggle,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "r",
                    green,
                    " Refresh",
                    raw,
                    FooterAction::Refresh,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
            }
            Panel::Files => {
                push_btn(
                    "Space",
                    bold_yellow,
                    " Stage",
                    raw,
                    FooterAction::StageToggle,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "Enter/l",
                    bold_cyan,
                    " Scroll Diff",
                    raw,
                    FooterAction::ScrollDiff,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "a",
                    green,
                    " All",
                    raw,
                    FooterAction::StageAll,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "c",
                    cyan,
                    " Commit",
                    raw,
                    FooterAction::Commit,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "s",
                    yellow,
                    " Stash",
                    raw,
                    FooterAction::StashSave,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "d",
                    bold_red,
                    " Discard",
                    raw,
                    FooterAction::Discard,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "PgDn",
                    cyan,
                    " Scroll",
                    raw,
                    FooterAction::PageInspectorDown,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
            }
            Panel::Branches => {
                push_btn(
                    "Space/Enter",
                    bold_yellow,
                    " Checkout",
                    raw,
                    FooterAction::CheckoutBranch,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "l",
                    cyan,
                    " Diff",
                    raw,
                    FooterAction::ScrollDiff,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "n",
                    green,
                    " New",
                    raw,
                    FooterAction::NewBranch,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "d",
                    bold_red,
                    " Delete",
                    raw,
                    FooterAction::DeleteBranch,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "[/]",
                    cyan,
                    " Tabs",
                    raw,
                    FooterAction::SwitchTab,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
            }
            Panel::Commits => {
                push_btn(
                    "Enter/l",
                    bold_cyan,
                    " Scroll Diff",
                    raw,
                    FooterAction::ScrollDiff,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "[/]",
                    cyan,
                    " Commits/Reflog",
                    raw,
                    FooterAction::SwitchTab,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "PgUp/PgDn",
                    cyan,
                    " Scroll",
                    raw,
                    FooterAction::PageInspectorDown,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
            }
            Panel::Stash => {
                push_btn(
                    "Space/Enter",
                    bold_yellow,
                    " Pop",
                    raw,
                    FooterAction::PopStash,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "l",
                    cyan,
                    " Diff",
                    raw,
                    FooterAction::ScrollDiff,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "a",
                    green,
                    " Apply",
                    raw,
                    FooterAction::ApplyStash,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "d",
                    bold_red,
                    " Drop",
                    raw,
                    FooterAction::DropStash,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
            }
        }
    }

    // Global hints
    push_btn(
        "P",
        bold_yellow,
        " Push",
        raw,
        FooterAction::Push,
        &mut footer_spans,
        &mut buttons,
        &mut col_offset,
    );
    push_btn(
        "p",
        cyan,
        " Pull",
        raw,
        FooterAction::Pull,
        &mut footer_spans,
        &mut buttons,
        &mut col_offset,
    );
    push_btn(
        "?",
        yellow,
        " Help",
        raw,
        FooterAction::Help,
        &mut footer_spans,
        &mut buttons,
        &mut col_offset,
    );

    // Quit (last item has no trailing separator)
    let start_x = col_offset;
    footer_spans.push(Span::styled("q", bold_red));
    footer_spans.push(Span::raw(" Quit"));
    let btn_len = ("q".len() + " Quit".len()) as u16;
    col_offset += btn_len;
    buttons.push((start_x, col_offset, FooterAction::Quit));

    (Line::from(footer_spans), buttons)
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let (line, _) = build_footer(app, area.width);
    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Rgb(15, 18, 22)));
    frame.render_widget(paragraph, area);
}

/// Computes the exact column bounds `(start_x, end_x)` for tabs in the Branches panel header.
pub fn branch_tab_ranges(app: &App, area: Rect) -> Vec<(BranchesTab, u16, u16)> {
    let title_prefix = " 3 Branches ";
    let mut current_x = area.x + 1 + title_prefix.len() as u16;
    let mut ranges = Vec::new();

    let tabs = [
        (BranchesTab::Local, "Local"),
        (BranchesTab::Remotes, "Remotes"),
        (BranchesTab::Tags, "Tags"),
    ];

    for (t, label) in tabs {
        let label_len = if app.branches_tab == t {
            format!("[ {} ]", label).len() as u16
        } else {
            format!(" {} ", label).len() as u16
        };
        let start_x = current_x;
        let end_x = start_x + label_len;
        ranges.push((t, start_x, end_x));
        current_x = end_x + 1;
    }
    ranges
}

/// Computes the exact column bounds `(start_x, end_x)` for tabs in the Commits panel header.
pub fn commit_tab_ranges(app: &App, area: Rect) -> Vec<(CommitsTab, u16, u16)> {
    let title_prefix = " 4 Commits ";
    let mut current_x = area.x + 1 + title_prefix.len() as u16;
    let mut ranges = Vec::new();

    let tabs = [
        (CommitsTab::Commits, "Commits"),
        (CommitsTab::Reflog, "Reflog"),
    ];

    for (t, label) in tabs {
        let label_len = if app.commits_tab == t {
            format!("[ {} ]", label).len() as u16
        } else {
            format!(" {} ", label).len() as u16
        };
        let start_x = current_x;
        let end_x = start_x + label_len;
        ranges.push((t, start_x, end_x));
        current_x = end_x + 1;
    }
    ranges
}

fn render_modals(frame: &mut Frame, app: &App) {
    match app.active_modal {
        ActiveModal::None => {}
        ActiveModal::CommitPrompt {
            ref message,
            cursor,
        } => {
            let area = centered_rect(65, 30, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " 💬 Commit Staged Changes (Enter: Submit, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
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
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Commit message:",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press Enter to commit, Esc to dismiss",
                    Style::default().fg(Color::DarkGray),
                )),
                v_chunks[2],
            );

            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::CommitAmend {
            ref message,
            cursor,
        } => {
            let area = centered_rect(65, 30, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    " ✏️ Amend Last Commit Message (Enter: Submit, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if message.is_empty() {
                Span::styled(
                    "Enter amended message...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(message, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Amended message:",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press Enter to amend commit, Esc to dismiss",
                    Style::default().fg(Color::DarkGray),
                )),
                v_chunks[2],
            );

            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::BranchCreate { ref name, cursor } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " 🌱 Create & Checkout New Branch (Enter: Create, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if name.is_empty() {
                Span::styled(
                    "Enter new branch name...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(name, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);

            frame.render_widget(
                Paragraph::new(Span::styled(
                    "New Branch Name:",
                    Style::default().fg(Color::Green),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::StashSave {
            ref message,
            cursor,
        } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightYellow))
                .title(Span::styled(
                    " 💾 Stash Working Directory Changes (Enter: Save, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if message.is_empty() {
                Span::styled(
                    "(optional) stash message...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(message, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);

            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Stash Message:",
                    Style::default().fg(Color::Yellow),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::Help => {
            let area = centered_rect(72, 80, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    format!(
                        " 📖 Keyboard Shortcuts Cheatsheet - {} (Esc: Close) ",
                        app.active_panel.title()
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let mut lines = vec![Line::from(Span::styled(
                format!("Shortcuts for Active Panel ({}):", app.active_panel.title()),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ))];

            match app.active_panel {
                Panel::Status => {
                    lines.push(Line::from(
                        "  r               Refresh repository status in place",
                    ));
                    lines.push(Line::from(
                        "  h / l           Toggle focus between sidebar and inspector",
                    ));
                }
                Panel::Files => {
                    lines.push(Line::from(
                        "  Space           Toggle stage / unstage for selected file",
                    ));
                    lines.push(Line::from(
                        "  a               Stage all changes / Unstage all if already staged",
                    ));
                    lines.push(Line::from(
                        "  c               Open commit dialog for staged changes",
                    ));
                    lines.push(Line::from(
                        "  A               Amend last commit with staged changes",
                    ));
                    lines.push(Line::from("  s               Stash working tree changes"));
                    lines.push(Line::from(
                        "  d               Discard unstaged modifications or delete untracked file",
                    ));
                }
                Panel::Branches => {
                    lines.push(Line::from(
                        "  Space / Enter   Switch / checkout selected branch",
                    ));
                    lines.push(Line::from(
                        "  n               Create and checkout new branch",
                    ));
                    lines.push(Line::from(
                        "  d               Delete selected branch (with safeguard)",
                    ));
                    lines.push(Line::from(
                        "  [ / ]           Switch sub-tabs (Local Branches ↔ Remotes ↔ Tags)",
                    ));
                }
                Panel::Commits => {
                    lines.push(Line::from(
                        "  Enter           Inspect commit details and parent diff",
                    ));
                    lines.push(Line::from(
                        "  [ / ]           Switch sub-tabs (Commits ↔ Reflog)",
                    ));
                    lines.push(Line::from("  PgUp / PgDn     Scroll commit diff"));
                }
                Panel::Stash => {
                    lines.push(Line::from(
                        "  Space / Enter   Pop selected stash into working directory",
                    ));
                    lines.push(Line::from(
                        "  a               Apply selected stash without dropping",
                    ));
                    lines.push(Line::from("  d               Drop selected stash entry"));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Global Navigation & Shortcuts:",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from("  1, 2, 3, 4, 5   Jump directly to panel (Status, Files, Branches, Commits, Stash)"));
            lines.push(Line::from(
                "  Tab / Shift+Tab Cycle forward / backward between panels",
            ));
            lines.push(Line::from(
                "  j / k (↓ / ↑)   Navigate items in focused panel (or scroll inspector)",
            ));
            lines.push(Line::from(
                "  h / l (← / →)   Switch focus between Sidebar and Main Inspector",
            ));
            lines.push(Line::from(
                "  [ / ]           Switch sub-tabs within active panel",
            ));
            lines.push(Line::from(
                "  PgUp / PgDn     Scroll Inspector diff pane up / down",
            ));
            lines.push(Line::from(
                "  Ctrl+u / Ctrl+d Fast half-page scroll in Inspector",
            ));
            lines.push(Line::from(
                "  r               Refresh repository state from disk",
            ));
            lines.push(Line::from(
                "  P               Push commits to remote (git/ox push)",
            ));
            lines.push(Line::from(
                "  p               Pull latest changes from remote (git/ox pull)",
            ));
            lines.push(Line::from("  ?               Toggle this help cheatsheet"));
            lines.push(Line::from(
                "  q / Esc         Exit LazyOx / close active popup",
            ));

            let p = Paragraph::new(lines).block(block);
            frame.render_widget(p, area);
        }
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

/// Computes the bounding rectangle for an active modal popup.
pub fn compute_modal_rect(modal: &ActiveModal, screen: Rect) -> Option<Rect> {
    match modal {
        ActiveModal::None => None,
        ActiveModal::CommitPrompt { .. } | ActiveModal::CommitAmend { .. } => {
            Some(centered_rect(65, 30, screen))
        }
        ActiveModal::BranchCreate { .. } | ActiveModal::StashSave { .. } => {
            Some(centered_rect(60, 25, screen))
        }
        ActiveModal::Help => Some(centered_rect(72, 80, screen)),
    }
}

/// Structured sub-regions of an active modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalLayout {
    pub area: Rect,
    pub input_rect: Option<Rect>,
    pub action_rect: Option<Rect>,
}

/// Computes inner input and action sub-rectangles for an active modal dialog.
pub fn compute_modal_layout(modal: &ActiveModal, screen: Rect) -> Option<ModalLayout> {
    let area = compute_modal_rect(modal, screen)?;
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);

    match modal {
        ActiveModal::CommitPrompt { .. } | ActiveModal::CommitAmend { .. } => {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);
            Some(ModalLayout {
                area,
                input_rect: Some(v_chunks[1]),
                action_rect: Some(v_chunks[2]),
            })
        }
        ActiveModal::BranchCreate { .. } | ActiveModal::StashSave { .. } => {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);
            Some(ModalLayout {
                area,
                input_rect: Some(v_chunks[1]),
                action_rect: None,
            })
        }
        ActiveModal::Help => Some(ModalLayout {
            area,
            input_rect: None,
            action_rect: None,
        }),
        ActiveModal::None => None,
    }
}
