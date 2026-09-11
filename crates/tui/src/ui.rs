//! Ratatui UI drawing functions and multi-panel layout rendering (LazyOx - authentic LazyGit replica).

use crate::app::App;
use crate::model::{ActiveModal, BranchesTab, CommitDecoration, CommitsTab, FocusedWindow, Panel};
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
            Constraint::Length(6),
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

    let mut lines = vec![
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

    if let Some(ref s) = app.sequencer_state {
        lines.push(Line::from(vec![
            Span::styled("Rebase: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "step {}/{} ({})",
                    s.current_step,
                    s.total_steps,
                    s.status.as_str()
                ),
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if app.bisect_state.is_active {
        let step_info = if app.bisect_state.remaining_steps > 0 {
            format!(" (~{} steps left)", app.bisect_state.remaining_steps)
        } else {
            String::new()
        };
        lines.push(Line::from(vec![
            Span::styled("Bisect: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("BISECTING{}", step_info),
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

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

    if let Some(ref query) = app.commit_search_filter {
        tab_spans.push(Span::styled(
            format!(" [/{}] ", query),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let block = get_panel_border(app, Panel::Commits, Line::from(tab_spans));

    let avail_height = (area.height as usize).saturating_sub(2);

    let items: Vec<ListItem> = match app.commits_tab {
        CommitsTab::Commits => {
            let indices = app.filtered_commit_indices();
            let cur_pos = indices
                .iter()
                .position(|&i| i == app.commits_selected)
                .unwrap_or(0);
            let (start_idx, end_idx) = window_range(indices.len(), cur_pos, avail_height);

            if indices.is_empty() {
                if app.commit_search_filter.is_some() {
                    vec![ListItem::new(Span::styled(
                        "  (no commits matching filter)",
                        Style::default().fg(Color::DarkGray),
                    ))]
                } else {
                    vec![ListItem::new(Span::styled(
                        "  (no commits in repository)",
                        Style::default().fg(Color::DarkGray),
                    ))]
                }
            } else {
                indices[start_idx..end_idx]
                    .iter()
                    .map(|&commit_idx| {
                        let c = &app.commits[commit_idx];
                        let is_selected = commit_idx == app.commits_selected;
                        let marker = if is_selected && is_focused {
                            "▶ "
                        } else if is_selected {
                            "▷ "
                        } else {
                            "  "
                        };

                        let lane_color = match c.lane % 5 {
                            0 => Color::Cyan,
                            1 => Color::Yellow,
                            2 => Color::Magenta,
                            3 => Color::Blue,
                            _ => Color::Red,
                        };

                        let graph_span = if c.graph_prefix.is_empty() {
                            Span::styled(
                                "* ",
                                Style::default().fg(lane_color).add_modifier(Modifier::BOLD),
                            )
                        } else {
                            Span::styled(
                                format!("{} ", c.graph_prefix),
                                Style::default().fg(lane_color),
                            )
                        };

                        let mut line_spans = vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            graph_span,
                            Span::styled(
                                &c.short_oid,
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                        ];

                        for deco in &c.decorations {
                            match deco {
                                CommitDecoration::Head(name) => {
                                    line_spans.push(Span::styled(
                                        format!("(HEAD -> {}) ", name),
                                        Style::default()
                                            .fg(Color::Green)
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                                CommitDecoration::Branch(name) => {
                                    line_spans.push(Span::styled(
                                        format!("({}) ", name),
                                        Style::default()
                                            .fg(Color::Cyan)
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                                CommitDecoration::Remote(name) => {
                                    line_spans.push(Span::styled(
                                        format!("({}) ", name),
                                        Style::default().fg(Color::Red),
                                    ));
                                }
                                CommitDecoration::Tag(name) => {
                                    line_spans.push(Span::styled(
                                        format!("(tag: {}) ", name),
                                        Style::default()
                                            .fg(Color::LightYellow)
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                            }
                        }

                        if app.bisect_state.is_active {
                            if app.bisect_state.culprit_oid == Some(c.oid) {
                                line_spans.push(Span::styled(
                                    "(culprit) ",
                                    Style::default()
                                        .fg(Color::LightRed)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            } else if app.bisect_state.bad_oid == Some(c.oid) {
                                line_spans.push(Span::styled(
                                    "(bad) ",
                                    Style::default()
                                        .fg(Color::LightRed)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            } else if app.bisect_state.good_oids.contains(&c.oid) {
                                line_spans.push(Span::styled(
                                    "(good) ",
                                    Style::default()
                                        .fg(Color::LightGreen)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            }
                        }

                        line_spans.push(Span::styled(
                            &c.summary,
                            Style::default().fg(if is_selected {
                                Color::White
                            } else {
                                Color::Gray
                            }),
                        ));
                        line_spans.push(Span::raw(" "));
                        line_spans.push(Span::styled(
                            format!("({})", c.author),
                            Style::default().fg(Color::DarkGray),
                        ));

                        let line = Line::from(line_spans);
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

    let has_hunks = app
        .cached_diff
        .as_ref()
        .is_some_and(|d| !d.hunks.is_empty());

    let basket_info = if !app.custom_patch_basket.is_empty() {
        format!(" [🧺 Basket: {}] ", app.custom_patch_basket.len())
    } else {
        String::new()
    };

    let hunk_info = if has_hunks {
        if let Some(diff) = &app.cached_diff {
            let total = diff.hunks.len();
            let current = diff.selected_hunk.map(|i| i + 1).unwrap_or(0);
            let stage_label = if diff.is_staged { "STAGED" } else { "UNSTAGED" };
            format!(" [Hunk {}/{} - {}] ", current, total, stage_label)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let title_text = if is_inspector_focused {
        if has_hunks {
            format!(
                " [ {} ]{}{}── [ [ / ] Hunk │ Space Stage │ a Basket │ P Patch Menu │ d Discard │ Esc/h Return ] ",
                base_title, hunk_info, basket_info
            )
        } else {
            format!(
                " [ {} ]{}── [ FOCUSED: j/k (or Mouse) Scroll │ PgDn/PgUp Page │ Esc/h Return ] ",
                base_title, basket_info
            )
        }
    } else {
        if has_hunks {
            format!(
                " [ {} ]{}{}── [ Enter/l Focus Hunks │ Space Toggle File ] ",
                base_title, hunk_info, basket_info
            )
        } else {
            format!(
                " [ {} ]{}── [ Enter/l to Focus & Scroll │ Mouse/PgDn to Scroll ] ",
                base_title, basket_info
            )
        }
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

    let initial_lang = match app.active_panel {
        Panel::Files => app
            .selected_file()
            .map(|f| crate::syntax::Language::from_path(&f.path))
            .unwrap_or_else(|| crate::syntax::Language::from_title(&diff_view.title)),
        _ => crate::syntax::Language::from_title(&diff_view.title),
    };
    let mut highlighter = crate::syntax::SyntaxHighlighter::new(initial_lang);
    let selected_hunk_idx = diff_view.selected_hunk;

    let avail_height = (area.height as usize).saturating_sub(2);
    let total_lines = diff_view.lines.len();
    let start_idx = app.inspector_scroll.min(total_lines);
    let end_idx = (start_idx + avail_height).min(total_lines);

    let lines: Vec<Line> = diff_view.lines[start_idx..end_idx]
        .iter()
        .enumerate()
        .map(|(offset, dl)| {
            let line_idx = start_idx + offset;
            let line_hunk = diff_view.hunk_at_line(line_idx);
            let is_in_selected = line_hunk.is_some() && line_hunk == selected_hunk_idx;
            let in_basket = if let (Some(path), Some(h_idx)) = (&diff_view.file_path, line_hunk) {
                h_idx < diff_view.hunks.len()
                    && app
                        .custom_patch_basket
                        .contains_hunk(path, &diff_view.hunks[h_idx])
            } else {
                false
            };
            let basket_tag = if in_basket { " [IN BASKET]" } else { "" };
            let line = highlighter.highlight_line(dl);

            if is_in_selected && is_inspector_focused {
                if dl.kind == crate::model::DiffLineKind::HunkHeader {
                    Line::from(Span::styled(
                        format!("▶ {} [ACTIVE HUNK]{}", dl.content.trim_end(), basket_tag),
                        Style::default()
                            .fg(Color::Yellow)
                            .bg(Color::Rgb(30, 45, 65))
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    let spans: Vec<Span<'static>> = line
                        .spans
                        .into_iter()
                        .map(|s| {
                            let mut style = s.style;
                            style = style.bg(Color::Rgb(25, 35, 50));
                            Span::styled(s.content, style)
                        })
                        .collect();
                    Line::from(spans)
                }
            } else if dl.kind == crate::model::DiffLineKind::HunkHeader && in_basket {
                Line::from(vec![
                    Span::styled(
                        dl.content.trim_end().to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        " [IN BASKET]".to_string(),
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                line
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines).block(block);

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
        let lang_badge = highlighter.current_lang.name();
        let indicator = format!(
            " [{} │ Line {}/{} - {}%] ",
            lang_badge, current_line, total_lines, scroll_pct
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
    PrevHunk,
    NextHunk,
    StageHunk,
    DiscardHunk,
    CheckoutCommit,
    CherryPick,
    ResetCommit,
    RenameBranch,
    FastForwardMerge,
    CreateTag,
    DeleteTag,
    SearchFilter,
    InteractiveRebase,
    RebaseContinue,
    RebaseSkip,
    RebaseAbort,
    RevertCommit,
    ResolveOurs,
    ResolveTheirs,
    ResolveBoth,
    AddToPatchBasket,
    CustomPatchMenu,
    StashBranch,
    WorktreeList,
}

/// Builds footer text spans and computes clickable bounding columns for footer actions.
pub fn build_footer(app: &App, _max_width: u16) -> (Line<'static>, Vec<(u16, u16, FooterAction)>) {
    let mut footer_spans = Vec::new();
    let mut buttons = Vec::new();
    let mut col_offset: u16 = 0;

    // Feedback message or default badge
    let badge_span = if let Some(ref msg) = app.status_message {
        Span::styled(
            format!(" {} ", msg),
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(40, 30, 20))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " [LazyOx] Ready ".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };
    let badge_width = badge_span.width() as u16;
    col_offset += badge_width;
    footer_spans.push(badge_span);
    footer_spans.push(Span::raw(" │ "));
    col_offset += 3;

    let push_btn = |key: &'static str,
                    key_style: Style,
                    label: &'static str,
                    label_style: Style,
                    action: FooterAction,
                    spans: &mut Vec<Span<'static>>,
                    btns: &mut Vec<(u16, u16, FooterAction)>,
                    offset: &mut u16| {
        let key_span = Span::styled(key, key_style);
        let label_span = Span::styled(label, label_style);
        let btn_width = (key_span.width() + label_span.width()) as u16;
        let sep_width = 3; // " │ "
        let start_x = *offset;
        let end_x = start_x + btn_width;
        spans.push(key_span);
        spans.push(label_span);
        *offset = end_x;
        btns.push((start_x, end_x, action));
        spans.push(Span::raw(" │ "));
        *offset += sep_width;
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
        if app.has_hunks() {
            push_btn(
                "[",
                bold_cyan,
                " Prev Hunk",
                raw,
                FooterAction::PrevHunk,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
            push_btn(
                "]",
                bold_cyan,
                " Next Hunk",
                raw,
                FooterAction::NextHunk,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
            let stage_label = if app.cached_diff.as_ref().is_some_and(|d| d.is_staged) {
                " Unstage Hunk"
            } else {
                " Stage Hunk"
            };
            push_btn(
                "Space",
                bold_yellow,
                stage_label,
                raw,
                FooterAction::StageHunk,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
            push_btn(
                "d",
                bold_red,
                " Discard Hunk",
                raw,
                FooterAction::DiscardHunk,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
            let in_basket = if let Some(diff) = &app.cached_diff {
                if let (Some(path), Some(idx)) = (&diff.file_path, diff.selected_hunk) {
                    if idx < diff.hunks.len() {
                        app.custom_patch_basket
                            .contains_hunk(path, &diff.hunks[idx])
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            let basket_label = if in_basket { " -Basket" } else { " +Basket" };
            push_btn(
                "a",
                bold_cyan,
                basket_label,
                raw,
                FooterAction::AddToPatchBasket,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
            push_btn(
                "P",
                bold_cyan,
                " Patch Menu",
                raw,
                FooterAction::CustomPatchMenu,
                &mut footer_spans,
                &mut buttons,
                &mut col_offset,
            );
        }
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
                if app.sequencer_state.is_some() {
                    push_btn(
                        "m",
                        bold_green,
                        " Continue Rebase",
                        raw,
                        FooterAction::RebaseContinue,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "s",
                        bold_yellow,
                        " Skip",
                        raw,
                        FooterAction::RebaseSkip,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "a",
                        bold_red,
                        " Abort",
                        raw,
                        FooterAction::RebaseAbort,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                }
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
                if app
                    .files
                    .get(app.files_selected)
                    .is_some_and(|f| f.kind == crate::model::FileStatusKind::Conflicted)
                {
                    push_btn(
                        "o",
                        bold_cyan,
                        " Ours",
                        raw,
                        FooterAction::ResolveOurs,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "t",
                        bold_yellow,
                        " Theirs",
                        raw,
                        FooterAction::ResolveTheirs,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "b",
                        bold_green,
                        " Both",
                        raw,
                        FooterAction::ResolveBoth,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                }
                if app.sequencer_state.is_some() {
                    push_btn(
                        "m",
                        bold_green,
                        " Continue Rebase",
                        raw,
                        FooterAction::RebaseContinue,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "s",
                        bold_yellow,
                        " Skip",
                        raw,
                        FooterAction::RebaseSkip,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "a",
                        bold_red,
                        " Abort",
                        raw,
                        FooterAction::RebaseAbort,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                }
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
            Panel::Branches => match app.branches_tab {
                BranchesTab::Local => {
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
                        "R",
                        cyan,
                        " Rename",
                        raw,
                        FooterAction::RenameBranch,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "f/M",
                        bold_yellow,
                        " Fast-forward",
                        raw,
                        FooterAction::FastForwardMerge,
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
                        "w",
                        bold_cyan,
                        " Worktrees",
                        raw,
                        FooterAction::WorktreeList,
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
                BranchesTab::Remotes => {
                    push_btn(
                        "f/M",
                        bold_yellow,
                        " Fast-forward",
                        raw,
                        FooterAction::FastForwardMerge,
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
                BranchesTab::Tags => {
                    push_btn(
                        "n",
                        green,
                        " New Tag",
                        raw,
                        FooterAction::CreateTag,
                        &mut footer_spans,
                        &mut buttons,
                        &mut col_offset,
                    );
                    push_btn(
                        "d",
                        bold_red,
                        " Delete Tag",
                        raw,
                        FooterAction::DeleteTag,
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
            },
            Panel::Commits => {
                push_btn(
                    "Space",
                    bold_yellow,
                    " Checkout",
                    raw,
                    FooterAction::CheckoutCommit,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "c",
                    cyan,
                    " Cherry-pick",
                    raw,
                    FooterAction::CherryPick,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "i",
                    bold_green,
                    " Rebase",
                    raw,
                    FooterAction::InteractiveRebase,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "t",
                    bold_yellow,
                    " Revert",
                    raw,
                    FooterAction::RevertCommit,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "g",
                    bold_red,
                    " Reset",
                    raw,
                    FooterAction::ResetCommit,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "n",
                    green,
                    " Tag",
                    raw,
                    FooterAction::CreateTag,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "/",
                    bold_yellow,
                    " Filter",
                    raw,
                    FooterAction::SearchFilter,
                    &mut footer_spans,
                    &mut buttons,
                    &mut col_offset,
                );
                push_btn(
                    "Enter/l",
                    bold_cyan,
                    " Diff",
                    raw,
                    FooterAction::ScrollDiff,
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
                push_btn(
                    "b",
                    bold_cyan,
                    " Branch",
                    raw,
                    FooterAction::StashBranch,
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
    let q_key = Span::styled("q", bold_red);
    let q_label = Span::raw(" Quit");
    let q_width = (q_key.width() + q_label.width()) as u16;
    let start_x = col_offset;
    col_offset += q_width;
    footer_spans.push(q_key);
    footer_spans.push(q_label);
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
                    " ✏️ Amend HEAD Commit (Enter: Submit, Esc: Cancel) ",
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
                    "Amended HEAD commit message:",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press Enter to amend HEAD commit, Esc to dismiss",
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
            include_untracked,
            staged_only,
            keep_index,
            focused_field,
        } => {
            let area = centered_rect(65, 35, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightYellow))
                .title(Span::styled(
                    " 💾 Stash Working Directory Changes (Enter: Save, Tab: Cycle, Space: Toggle, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // label
                    Constraint::Length(1), // input
                    Constraint::Length(1), // spacing
                    Constraint::Length(1), // option 1: untracked
                    Constraint::Length(1), // option 2: staged only
                    Constraint::Length(1), // option 3: keep index
                ])
                .split(inner);

            let msg_style = if focused_field == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            frame.render_widget(
                Paragraph::new(Span::styled("Stash Message:", msg_style)),
                v_chunks[0],
            );

            let display_text = if message.is_empty() {
                Span::styled(
                    "(optional) stash message...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(message, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            let render_checkbox =
                |label: &'static str, checked: bool, is_focused: bool| -> Line<'static> {
                    let box_char = if checked { "[x] " } else { "[ ] " };
                    let (box_style, text_style) = if is_focused {
                        (
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else if checked {
                        (
                            Style::default().fg(Color::Green),
                            Style::default().fg(Color::White),
                        )
                    } else {
                        (
                            Style::default().fg(Color::DarkGray),
                            Style::default().fg(Color::Gray),
                        )
                    };
                    let indicator = if is_focused { "▶ " } else { "  " };
                    Line::from(vec![
                        Span::styled(indicator, Style::default().fg(Color::Cyan)),
                        Span::styled(box_char, box_style),
                        Span::styled(label, text_style),
                    ])
                };

            frame.render_widget(
                Paragraph::new(render_checkbox(
                    "Include Untracked (-u / --include-untracked)",
                    include_untracked,
                    focused_field == 1,
                )),
                v_chunks[3],
            );
            frame.render_widget(
                Paragraph::new(render_checkbox(
                    "Staged Only (--staged)",
                    staged_only,
                    focused_field == 2,
                )),
                v_chunks[4],
            );
            frame.render_widget(
                Paragraph::new(render_checkbox(
                    "Keep Index (--keep-index)",
                    keep_index,
                    focused_field == 3,
                )),
                v_chunks[5],
            );

            if focused_field == 0 {
                frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
            }
        }
        ActiveModal::BranchRename {
            ref old_name,
            ref new_name,
            cursor,
        } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(
                        " ✏️ Rename Branch '{}' (Enter: Rename, Esc: Cancel) ",
                        old_name
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if new_name.is_empty() {
                Span::styled(
                    "Enter new branch name...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(new_name, Style::default().fg(Color::White))
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
        ActiveModal::TagCreate {
            ref target_oid,
            ref name,
            cursor,
        } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let short = &target_oid.to_string()[..7];
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" 🏷️ Create Tag at {} (Enter: Create, Esc: Cancel) ", short),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if name.is_empty() {
                Span::styled(
                    "Enter tag name (e.g. v1.0.0)...",
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
                    "Tag Name:",
                    Style::default().fg(Color::Yellow),
                )),
                v_chunks[0],
            );
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);

            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::SearchFilter { ref query, cursor } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    " 🔍 Filter Commits (Enter: Apply, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let display_text = if query.is_empty() {
                Span::styled(
                    "Type query to filter commits...",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(query, Style::default().fg(Color::White))
            };

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(inner);

            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Filter Query:",
                    Style::default().fg(Color::Magenta),
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
                    lines.push(Line::from("  R               Rename selected local branch"));
                    lines.push(Line::from(
                        "  f / M           Fast-forward merge branch into HEAD",
                    ));
                    lines.push(Line::from(
                        "  n               Create and checkout new branch / tag",
                    ));
                    lines.push(Line::from("  d               Delete selected branch / tag"));
                    lines.push(Line::from(
                        "  [ / ]           Switch sub-tabs (Local Branches ↔ Remotes ↔ Tags)",
                    ));
                }
                Panel::Commits => {
                    lines.push(Line::from(
                        "  Space           Checkout commit in detached HEAD state",
                    ));
                    lines.push(Line::from(
                        "  c               Cherry-pick commit onto current branch",
                    ));
                    lines.push(Line::from(
                        "  g / G           Reset HEAD to commit (Mixed / Hard)",
                    ));
                    lines.push(Line::from(
                        "  n               Create lightweight tag at commit",
                    ));
                    lines.push(Line::from(
                        "  /               Filter commits by summary, SHA, or author",
                    ));
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
        ActiveModal::Confirm {
            ref title,
            ref prompt,
            ..
        } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightRed))
                .title(Span::styled(
                    format!(" ⚠️ {} ", title),
                    Style::default()
                        .fg(Color::LightRed)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);

            let p = Paragraph::new(prompt.as_str()).style(Style::default().fg(Color::White));
            frame.render_widget(p, v_chunks[0]);

            let button_spans = Line::from(vec![
                Span::styled(
                    " [y] Confirm ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::LightRed)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled(
                    " [n / Esc] Cancel ",
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(button_spans), v_chunks[1]);
        }
        ActiveModal::RebaseTodo {
            ref items,
            selected,
            onto_oid,
        } => {
            let area = centered_rect(75, 70, frame.area());
            frame.render_widget(Clear, area);

            let onto_short = if onto_oid.to_string().len() >= 7 {
                &onto_oid.to_string()[..7]
            } else {
                &onto_oid.to_string()
            };

            let title = format!(
                " 🔄 Interactive Rebase (onto {}) (Enter: Start, Esc: Abort) ",
                onto_short
            );
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(2)])
                .split(inner);

            let list_items: Vec<ListItem> = items
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let is_sel = idx == selected;
                    let (action_str, action_color) = match item.action {
                        crate::sequencer::RebaseAction::Pick => ("pick  ", Color::Green),
                        crate::sequencer::RebaseAction::Reword => ("reword", Color::Cyan),
                        crate::sequencer::RebaseAction::Edit => ("edit  ", Color::Yellow),
                        crate::sequencer::RebaseAction::Squash => ("squash", Color::Magenta),
                        crate::sequencer::RebaseAction::Fixup => ("fixup ", Color::Blue),
                        crate::sequencer::RebaseAction::Drop => ("drop  ", Color::Red),
                    };

                    let prefix = if is_sel { "▶ " } else { "  " };
                    let style = if is_sel {
                        Style::default()
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let spans = Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Yellow)),
                        Span::styled(
                            format!("[{}] ", action_str),
                            Style::default()
                                .fg(action_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} ", item.short_oid),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            item.summary.clone(),
                            if is_sel {
                                Style::default().fg(Color::White)
                            } else {
                                Style::default().fg(Color::Gray)
                            },
                        ),
                    ]);
                    ListItem::new(spans).style(style)
                })
                .collect();

            let list = List::new(list_items);
            frame.render_widget(list, v_chunks[0]);

            let help_spans = Line::from(vec![
                Span::styled(" [p]ick ", Style::default().fg(Color::Green)),
                Span::styled("[r]eword ", Style::default().fg(Color::Cyan)),
                Span::styled("[e]dit ", Style::default().fg(Color::Yellow)),
                Span::styled("[s]quash ", Style::default().fg(Color::Magenta)),
                Span::styled("[f]fixup ", Style::default().fg(Color::Blue)),
                Span::styled("[d]rop ", Style::default().fg(Color::Red)),
                Span::styled("[J/K] move ", Style::default().fg(Color::White)),
                Span::styled("[Space] cycle ", Style::default().fg(Color::White)),
                Span::styled(
                    " [Enter] Start ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(help_spans), v_chunks[1]);
        }
        ActiveModal::StashBranch {
            stash_idx,
            ref branch_name,
            cursor,
        } => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(
                        " 🌿 Branch from stash@{{{}}} (Enter: Create & Apply, Esc: Cancel) ",
                        stash_idx
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

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

            let display_text = if branch_name.is_empty() {
                Span::styled("Enter branch name...", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(branch_name, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(display_text), v_chunks[1]);
            frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
        }
        ActiveModal::CustomPatchMenu { selected } => {
            let area = centered_rect(60, 40, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    " 🧩 Custom Patch Menu (Enter: Select, Esc: Close) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let items = [
                "[1] Apply custom patch to working tree",
                "[2] Apply custom patch to index (staging)",
                "[3] Revert custom patch from working tree",
                "[4] Revert custom patch from index",
                "[5] Create commit from custom patch",
                "[6] Clear custom patch basket",
            ];

            let list_items: Vec<ListItem> = items
                .iter()
                .enumerate()
                .map(|(idx, &label)| {
                    let is_sel = idx == selected;
                    let prefix = if is_sel { "▶ " } else { "  " };
                    let style = if is_sel {
                        Style::default()
                            .bg(Color::DarkGray)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    let spans = Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Yellow)),
                        Span::styled(
                            label,
                            if is_sel {
                                Style::default().fg(Color::White)
                            } else {
                                Style::default().fg(Color::Gray)
                            },
                        ),
                    ]);
                    ListItem::new(spans).style(style)
                })
                .collect();

            let list = List::new(list_items);
            frame.render_widget(list, inner);
        }
        ActiveModal::WorktreeList {
            ref items,
            selected,
        } => {
            let area = centered_rect(75, 60, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightBlue))
                .title(Span::styled(
                    " 🌳 Linked Worktrees (Enter: Switch, a/n: Add, d: Remove, Esc: Close) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(2)])
                .split(inner);

            let list_items: Vec<ListItem> = items
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let is_sel = idx == selected;
                    let prefix = if is_sel { "▶ " } else { "  " };
                    let style = if is_sel {
                        Style::default()
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let badge = if item.is_main {
                        Span::styled(
                            "[MAIN] ",
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else if item.is_locked {
                        Span::styled(
                            "[LOCKED] ",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        )
                    } else if item.is_prunable {
                        Span::styled("[PRUNABLE] ", Style::default().fg(Color::DarkGray))
                    } else {
                        Span::styled("[LINKED] ", Style::default().fg(Color::Cyan))
                    };

                    let branch_span = Span::styled(
                        format!("{:<15} ", item.head_ref),
                        Style::default().fg(Color::Yellow),
                    );

                    let path_span = Span::styled(
                        item.path.display().to_string(),
                        if is_sel {
                            Style::default().fg(Color::White)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    );

                    let spans = Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Yellow)),
                        badge,
                        branch_span,
                        path_span,
                    ]);
                    ListItem::new(spans).style(style)
                })
                .collect();

            let list = List::new(list_items);
            frame.render_widget(list, v_chunks[0]);

            let help_spans = Line::from(vec![
                Span::styled(
                    " [Enter] Switch worktree ",
                    Style::default().fg(Color::Green),
                ),
                Span::styled(" [a/n] Add worktree ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    " [d] Remove linked worktree ",
                    Style::default().fg(Color::Red),
                ),
                Span::styled(" [Esc] Close ", Style::default().fg(Color::DarkGray)),
            ]);
            frame.render_widget(Paragraph::new(help_spans), v_chunks[1]);
        }
        ActiveModal::WorktreeAdd {
            ref path,
            ref branch,
            create_branch,
            focused_field,
            cursor,
        } => {
            let area = centered_rect(65, 35, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightCyan))
                .title(Span::styled(
                    " ➕ Add Linked Worktree (Tab: Cycle, Space: Toggle, Enter: Create, Esc: Cancel) ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // path label
                    Constraint::Length(1), // path input
                    Constraint::Length(1), // branch label
                    Constraint::Length(1), // branch input
                    Constraint::Length(1), // spacing
                    Constraint::Length(1), // create_branch checkbox
                ])
                .split(inner);

            let p_lbl = if focused_field == 0 {
                Span::styled(
                    "Worktree Target Path:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    "Worktree Target Path:",
                    Style::default().fg(Color::DarkGray),
                )
            };
            frame.render_widget(Paragraph::new(p_lbl), v_chunks[0]);

            let p_txt = if path.is_empty() {
                Span::styled(
                    "e.g. ../feature-worktree",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(path, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(p_txt), v_chunks[1]);

            let b_lbl = if focused_field == 1 {
                Span::styled(
                    "Branch Name:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("Branch Name:", Style::default().fg(Color::DarkGray))
            };
            frame.render_widget(Paragraph::new(b_lbl), v_chunks[2]);

            let b_txt = if branch.is_empty() {
                Span::styled("e.g. feature-x", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(branch, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(b_txt), v_chunks[3]);

            let box_char = if create_branch { "[x] " } else { "[ ] " };
            let (box_style, text_style) = if focused_field == 2 {
                (
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::Gray),
                )
            };
            let indicator = if focused_field == 2 { "▶ " } else { "  " };
            let cb_line = Line::from(vec![
                Span::styled(indicator, Style::default().fg(Color::Cyan)),
                Span::styled(box_char, box_style),
                Span::styled("Create new branch (-b)", text_style),
            ]);
            frame.render_widget(Paragraph::new(cb_line), v_chunks[5]);

            if focused_field == 0 {
                frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
            } else if focused_field == 1 {
                frame.set_cursor_position((v_chunks[3].x + cursor as u16, v_chunks[3].y));
            }
        }
        ActiveModal::RemoteAdd {
            ref name,
            ref url,
            focused_field,
            cursor,
        } => {
            let area = centered_rect(65, 35, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " 🌐 Add Remote (Enter: Save, Tab: Switch, Esc: Cancel) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .split(inner);

            let name_style = if focused_field == 0 {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            frame.render_widget(
                Paragraph::new(Span::styled("Remote Name:", name_style)),
                v_chunks[0],
            );
            let name_display = if name.is_empty() {
                Span::styled("e.g. origin", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(name, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(name_display), v_chunks[1]);

            let url_style = if focused_field == 1 {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            frame.render_widget(
                Paragraph::new(Span::styled("Remote URL:", url_style)),
                v_chunks[3],
            );
            let url_display = if url.is_empty() {
                Span::styled(
                    "e.g. git@github.com:user/repo.git",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(url, Style::default().fg(Color::White))
            };
            frame.render_widget(Paragraph::new(url_display), v_chunks[4]);

            let hint = Line::from(vec![
                Span::styled("Tab: ", Style::default().fg(Color::Cyan)),
                Span::raw("Next field | "),
                Span::styled("Enter: ", Style::default().fg(Color::Green)),
                Span::raw("Confirm | "),
                Span::styled("Esc: ", Style::default().fg(Color::Red)),
                Span::raw("Cancel"),
            ]);
            frame.render_widget(Paragraph::new(hint), v_chunks[5]);

            if focused_field == 0 {
                frame.set_cursor_position((v_chunks[1].x + cursor as u16, v_chunks[1].y));
            } else if focused_field == 1 {
                frame.set_cursor_position((v_chunks[4].x + cursor as u16, v_chunks[4].y));
            }
        }
        ActiveModal::SubmoduleList {
            ref items,
            selected,
        } => {
            let area = centered_rect(75, 60, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " 📦 Submodules (Enter: Enter repo, u: Update, i: Init, Esc: Close) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            if items.is_empty() {
                let msg = Paragraph::new("No submodules configured in .gitmodules")
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(msg, inner);
            } else {
                let list_items: Vec<ListItem> = items
                    .iter()
                    .enumerate()
                    .map(|(idx, sub)| {
                        let is_sel = idx == selected;
                        let marker = if is_sel { "▶ " } else { "  " };
                        let status_span = if sub.is_initialized {
                            Span::styled("[init] ", Style::default().fg(Color::Green))
                        } else {
                            Span::styled("[uninit] ", Style::default().fg(Color::DarkGray))
                        };
                        let dirty_span = if sub.is_dirty {
                            Span::styled(
                                "[dirty] ",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            Span::raw("")
                        };
                        let oid_span = if let Some(oid) = sub.head_oid {
                            Span::styled(
                                format!("({}) ", &oid.to_string()[..7]),
                                Style::default().fg(Color::Yellow),
                            )
                        } else {
                            Span::raw("")
                        };

                        let style = if is_sel {
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(30, 60, 45))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            status_span,
                            dirty_span,
                            Span::styled(
                                &sub.name,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" -> "),
                            Span::styled(&sub.path, Style::default().fg(Color::White)),
                            Span::raw(" "),
                            oid_span,
                            Span::styled(
                                format!("<{}>", sub.url),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]);
                        ListItem::new(line).style(style)
                    })
                    .collect();

                let list = List::new(list_items);
                frame.render_widget(list, inner);
            }
        }
        ActiveModal::BisectMenu {
            ref state,
            selected,
        } => {
            let area = centered_rect(60, 40, frame.area());
            frame.render_widget(Clear, area);

            let title = if state.is_active {
                " 🎯 Git Bisect Controls (Active Session) "
            } else {
                " 🎯 Git Bisect Controls "
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(1)])
                .split(inner);

            let status_line = if state.is_active {
                Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        "BISECTING",
                        Style::default()
                            .fg(Color::LightMagenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(" | Remaining ~{} step(s)", state.remaining_steps)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
                    Span::styled("INACTIVE", Style::default().fg(Color::Gray)),
                ])
            };
            frame.render_widget(Paragraph::new(status_line), v_chunks[0]);

            let options = if state.is_active {
                vec![
                    "1. Mark Current HEAD as Bad (git bisect bad)",
                    "2. Mark Current HEAD as Good (git bisect good)",
                    "3. Skip Current Commit (git bisect skip)",
                    "4. Reset / Abort Bisect (git bisect reset)",
                ]
            } else {
                vec![
                    "1. Start Bisect with Current HEAD as Bad",
                    "2. Mark Commit as Good",
                ]
            };

            let list_items: Vec<ListItem> = options
                .iter()
                .enumerate()
                .map(|(idx, opt)| {
                    let is_sel = idx == selected;
                    let marker = if is_sel { "▶ " } else { "  " };
                    let style = if is_sel {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Green)),
                        Span::styled(*opt, style),
                    ]))
                })
                .collect();

            let list = List::new(list_items);
            frame.render_widget(list, v_chunks[1]);
        }
        ActiveModal::CommandPalette {
            ref query,
            cursor,
            selected,
            ref commands,
        } => {
            let area = centered_rect(70, 60, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " 🔍 Command Palette (Esc: Close, Enter: Execute, ↑/↓: Select) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(1)])
                .split(inner);

            let input_line = Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    query,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(input_line), v_chunks[0]);
            frame.set_cursor_position((v_chunks[0].x + 2 + cursor as u16, v_chunks[0].y));

            if commands.is_empty() {
                let no_res = Paragraph::new("No matching commands found")
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(no_res, v_chunks[1]);
            } else {
                let avail_height = v_chunks[1].height as usize;
                let (start_idx, end_idx) = window_range(commands.len(), selected, avail_height);

                let list_items: Vec<ListItem> = commands[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, cmd)| {
                        let actual_idx = start_idx + offset;
                        let is_sel = actual_idx == selected;
                        let marker = if is_sel { "▶ " } else { "  " };

                        let style = if is_sel {
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(30, 60, 45))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                format!("[{}] ", cmd.category),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                cmd.title,
                                Style::default().fg(if is_sel {
                                    Color::White
                                } else {
                                    Color::Gray
                                }),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("({})", cmd.keybinding),
                                Style::default().fg(Color::Yellow),
                            ),
                        ]);
                        ListItem::new(line).style(style)
                    })
                    .collect();

                let list = List::new(list_items);
                frame.render_widget(list, v_chunks[1]);
            }
        }
        ActiveModal::ProviderLinks { ref urls, selected } => {
            let area = centered_rect(65, 35, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " 🌐 Web Provider Links (Enter: Copy Link, Esc: Close) ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            let list = [
                ("Repository", urls.repo_url.as_deref()),
                ("Commit", urls.commit_url.as_deref()),
                ("Branch", urls.branch_url.as_deref()),
                ("Pull Request", urls.pr_url.as_deref()),
            ];
            let available: Vec<(&str, &str)> = list
                .iter()
                .filter_map(|(l, u)| u.map(|url| (*l, url)))
                .collect();

            if available.is_empty() {
                let msg = Paragraph::new("No remote web links detected for this repository")
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(msg, inner);
            } else {
                let list_items: Vec<ListItem> = available
                    .iter()
                    .enumerate()
                    .map(|(idx, (label, url))| {
                        let is_sel = idx == selected;
                        let marker = if is_sel { "▶ " } else { "  " };
                        let style = if is_sel {
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Rgb(30, 60, 45))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };

                        let line = Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(
                                format!("{}: ", label),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(*url, Style::default().fg(Color::White)),
                        ]);
                        ListItem::new(line).style(style)
                    })
                    .collect();

                let list = List::new(list_items);
                frame.render_widget(list, inner);
            }
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
        ActiveModal::BranchCreate { .. }
        | ActiveModal::BranchRename { .. }
        | ActiveModal::TagCreate { .. }
        | ActiveModal::SearchFilter { .. }
        | ActiveModal::StashSave { .. }
        | ActiveModal::StashBranch { .. }
        | ActiveModal::Confirm { .. } => Some(centered_rect(60, 25, screen)),
        ActiveModal::CustomPatchMenu { .. } => Some(centered_rect(60, 40, screen)),
        ActiveModal::WorktreeList { .. } => Some(centered_rect(75, 60, screen)),
        ActiveModal::WorktreeAdd { .. } => Some(centered_rect(65, 35, screen)),
        ActiveModal::RemoteAdd { .. } => Some(centered_rect(65, 35, screen)),
        ActiveModal::SubmoduleList { .. } => Some(centered_rect(75, 60, screen)),
        ActiveModal::BisectMenu { .. } => Some(centered_rect(60, 40, screen)),
        ActiveModal::CommandPalette { .. } => Some(centered_rect(70, 60, screen)),
        ActiveModal::ProviderLinks { .. } => Some(centered_rect(65, 35, screen)),
        ActiveModal::Help => Some(centered_rect(72, 80, screen)),
        ActiveModal::RebaseTodo { .. } => Some(centered_rect(75, 70, screen)),
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
        ActiveModal::BranchCreate { .. }
        | ActiveModal::BranchRename { .. }
        | ActiveModal::TagCreate { .. }
        | ActiveModal::SearchFilter { .. }
        | ActiveModal::StashSave { .. }
        | ActiveModal::StashBranch { .. }
        | ActiveModal::WorktreeAdd { .. }
        | ActiveModal::RemoteAdd { .. } => {
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
        ActiveModal::Confirm { .. } => {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);
            Some(ModalLayout {
                area,
                input_rect: None,
                action_rect: Some(v_chunks[1]),
            })
        }
        ActiveModal::Help => Some(ModalLayout {
            area,
            input_rect: None,
            action_rect: None,
        }),
        ActiveModal::RebaseTodo { .. }
        | ActiveModal::CustomPatchMenu { .. }
        | ActiveModal::WorktreeList { .. }
        | ActiveModal::SubmoduleList { .. }
        | ActiveModal::BisectMenu { .. }
        | ActiveModal::CommandPalette { .. }
        | ActiveModal::ProviderLinks { .. } => Some(ModalLayout {
            area,
            input_rect: Some(inner),
            action_rect: None,
        }),
        ActiveModal::None => None,
    }
}
