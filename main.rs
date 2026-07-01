use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Application, Bounds, Context, MouseDownEvent, MouseMoveEvent, PathBuilder, Pixels, Point,
    Render, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, rgb,
};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source, source::SineWave};

const RADIUS: i32 = 4;
const HEX_SIZE: f32 = 34.0;
const DIRECTIONS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
const FLIP_DURATION: Duration = Duration::from_millis(420);
const COMPUTER_DELAY_MIN_MS: u64 = 1_100;
const COMPUTER_DELAY_MAX_MS: u64 = 2_400;
const COMPUTER_REPLY_DELAY_MIN_MS: u64 = 450;
const COMPUTER_REPLY_DELAY_MAX_MS: u64 = 900;
const SPEED_DELAY_MIN_MS: u64 = 80;
const SPEED_DELAY_MAX_MS: u64 = 180;

struct Audio {
    sink: Option<MixerDeviceSink>,
}

impl Audio {
    fn new() -> Self {
        let sink = DeviceSinkBuilder::open_default_sink()
            .map(|mut sink| {
                sink.log_on_drop(false);
                sink
            })
            .ok();

        Self { sink }
    }

    fn move_sound(&self) {
        self.play_tone(520.0, 46, 0.18, 0);
        self.play_tone(780.0, 54, 0.12, 28);
    }

    fn computer_sound(&self) {
        self.play_tone(360.0, 42, 0.14, 0);
        self.play_tone(540.0, 48, 0.14, 34);
    }

    fn invalid_sound(&self) {
        self.play_tone(150.0, 90, 0.16, 0);
    }

    fn hover_sound(&self) {
        self.play_tone(620.0, 32, 0.09, 0);
    }

    fn selection_sound(&self) {
        self.play_tone(700.0, 24, 0.07, 0);
    }

    fn game_over_sound(&self) {
        self.play_tone(440.0, 90, 0.15, 0);
        self.play_tone(660.0, 110, 0.15, 95);
        self.play_tone(880.0, 140, 0.13, 205);
    }

    fn new_game_sound(&self) {
        self.play_tone(660.0, 55, 0.13, 0);
        self.play_tone(990.0, 75, 0.11, 45);
    }

    fn play_tone(&self, frequency: f32, duration_ms: u64, volume: f32, delay_ms: u64) {
        let Some(sink) = &self.sink else {
            return;
        };

        let tone = SineWave::new(frequency)
            .take_duration(Duration::from_millis(duration_ms))
            .fade_in(Duration::from_millis(5))
            .fade_out(Duration::from_millis(18))
            .amplify(volume)
            .delay(Duration::from_millis(delay_ms));
        sink.mixer().add(tone);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Player {
    Black,
    White,
}

impl Player {
    fn other(self) -> Self {
        match self {
            Player::Black => Player::White,
            Player::White => Player::Black,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Player::Black => "Black",
            Player::White => "White",
        }
    }
}

#[derive(Clone)]
struct CellGeometry {
    q: i32,
    r: i32,
    center: Point<Pixels>,
}

#[derive(Clone)]
struct BoardGeometry {
    cells: Vec<CellGeometry>,
    hex_size: f32,
}

#[derive(Clone)]
struct FlipCell {
    q: i32,
    r: i32,
    from: Player,
    to: Player,
}

#[derive(Clone)]
struct FlipAnimation {
    cells: Vec<FlipCell>,
    start: Instant,
}

#[derive(Clone, Copy)]
struct MoveScore {
    q: i32,
    r: i32,
    score: i32,
}

#[derive(Clone)]
struct Game {
    board: HashMap<(i32, i32), Player>,
    current: Player,
    computer: Player,
    game_over: bool,
    message: Option<String>,
    flip_animation: Option<FlipAnimation>,
}

impl Game {
    fn new() -> Self {
        let mut board = HashMap::new();
        board.insert((0, 0), Player::White);
        board.insert((1, -1), Player::Black);
        board.insert((0, -1), Player::Black);
        board.insert((1, 0), Player::White);

        let computer = random_player();
        Self {
            board,
            current: Player::Black,
            computer,
            game_over: false,
            message: None,
            flip_animation: None,
        }
    }

    fn play_human(&mut self, q: i32, r: i32) -> bool {
        if self.game_over {
            return false;
        }

        if self.current == self.computer {
            self.message = Some("Computer is thinking.".to_string());
            return false;
        }

        let flips = self.flips_for_move(q, r, self.current);
        if flips.is_empty() {
            self.message = Some("That move does not trap any stones.".to_string());
            return false;
        }

        self.apply_move(q, r, flips);
        true
    }

    fn play_computer_move(&mut self) -> bool {
        if self.game_over || self.current != self.computer {
            return false;
        }

        let Some(choice) = self.choose_computer_move() else {
            self.advance_past_passes();
            return false;
        };
        let flips = self.flips_for_move(choice.q, choice.r, self.current);
        if flips.is_empty() {
            return false;
        }

        self.apply_move(choice.q, choice.r, flips);
        if !self.game_over {
            self.message = Some("Computer played.".to_string());
        }
        true
    }

    fn apply_move(&mut self, q: i32, r: i32, flips: Vec<(i32, i32)>) {
        let player = self.current;
        self.board.insert((q, r), player);
        for cell in &flips {
            self.board.insert(*cell, player);
        }
        self.flip_animation = Some(FlipAnimation {
            cells: flips
                .into_iter()
                .map(|(q, r)| FlipCell {
                    q,
                    r,
                    from: player.other(),
                    to: player,
                })
                .collect(),
            start: Instant::now(),
        });

        self.current = self.current.other();
        self.message = None;
        self.advance_past_passes();
    }

    fn advance_past_passes(&mut self) {
        if !self.legal_moves(self.current).is_empty() {
            return;
        }

        let waiting = self.current;
        let next = self.current.other();
        if self.legal_moves(next).is_empty() {
            self.game_over = true;
            return;
        }

        self.current = next;
        self.message = Some(format!("{} has no legal moves and passes.", waiting.name()));
    }

    fn flips_for_move(&self, q: i32, r: i32, player: Player) -> Vec<(i32, i32)> {
        if self.board.contains_key(&(q, r)) || !is_on_board(q, r) {
            return Vec::new();
        }

        let opponent = player.other();
        let mut flips = Vec::new();

        for (dq, dr) in DIRECTIONS {
            let mut line = Vec::new();
            let mut cq = q + dq;
            let mut cr = r + dr;

            while is_on_board(cq, cr) && self.board.get(&(cq, cr)) == Some(&opponent) {
                line.push((cq, cr));
                cq += dq;
                cr += dr;
            }

            if !line.is_empty() && self.board.get(&(cq, cr)) == Some(&player) {
                flips.extend(line);
            }
        }

        flips
    }

    fn legal_moves(&self, player: Player) -> Vec<(i32, i32)> {
        cells()
            .into_iter()
            .filter(|(q, r)| !self.flips_for_move(*q, *r, player).is_empty())
            .collect()
    }

    fn scored_moves(&self, player: Player) -> Vec<MoveScore> {
        let mut moves = self
            .legal_moves(player)
            .into_iter()
            .map(|(q, r)| MoveScore {
                q,
                r,
                score: self.move_score(q, r, player),
            })
            .collect::<Vec<_>>();
        moves.sort_by_key(|move_score| -move_score.score);
        moves
    }

    fn choose_computer_move(&self) -> Option<MoveScore> {
        let moves = self.scored_moves(self.current);
        let best_score = moves.first()?.score;
        let best = moves
            .iter()
            .copied()
            .filter(|move_score| move_score.score == best_score)
            .collect::<Vec<_>>();
        Some(best[random_index(best.len())])
    }

    fn move_score(&self, q: i32, r: i32, player: Player) -> i32 {
        let flips = self.flips_for_move(q, r, player).len() as i32;
        let s = -q - r;
        let ring = q.abs().max(r.abs()).max(s.abs());
        let is_corner = ring == RADIUS && (q == 0 || r == 0 || s == 0);
        let is_edge = ring == RADIUS;

        flips * 100
            + if is_corner {
                70
            } else if is_edge {
                30
            } else {
                0
            }
            - ring * 2
    }

    fn score(&self) -> (usize, usize) {
        let black = self.board.values().filter(|&&p| p == Player::Black).count();
        let white = self.board.values().filter(|&&p| p == Player::White).count();
        (black, white)
    }

    fn status(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }

        let (black, white) = self.score();
        if self.game_over {
            return match black.cmp(&white) {
                std::cmp::Ordering::Greater => format!(
                    "Black {black} > White {white}: {} won",
                    if self.computer == Player::Black {
                        "Computer"
                    } else {
                        "Human"
                    }
                ),
                std::cmp::Ordering::Less => format!(
                    "White {white} > Black {black}: {} won",
                    if self.computer == Player::White {
                        "Computer"
                    } else {
                        "Human"
                    }
                ),
                std::cmp::Ordering::Equal => format!("Black {black} = White {white}: Draw"),
            };
        }

        if self.current == self.computer {
            "Computer is thinking.".to_string()
        } else {
            format!("{} to move", self.current.name())
        }
    }
}

struct HexReversi {
    game: Game,
    geometry: Rc<RefCell<Option<BoardGeometry>>>,
    hover_cell: Option<(i32, i32)>,
    speed_mode: bool,
    sound_enabled: bool,
    audio: Audio,
    turn_serial: u64,
}

impl HexReversi {
    fn new() -> Self {
        Self {
            game: Game::new(),
            geometry: Rc::new(RefCell::new(None)),
            hover_cell: None,
            speed_mode: false,
            sound_enabled: true,
            audio: Audio::new(),
            turn_serial: 0,
        }
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.game = Game::new();
        self.hover_cell = None;
        self.turn_serial += 1;
        if self.sound_enabled {
            self.audio.new_game_sound();
        }
        self.schedule_computer_turn(cx, false);
        cx.notify();
    }

    fn click_board(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some((q, r)) = self.cell_at_position(event.position) {
            let human_turn = !self.game.game_over && self.game.current != self.game.computer;
            let legal = human_turn && !self.game.flips_for_move(q, r, self.game.current).is_empty();
            let was_game_over = self.game.game_over;

            if self.game.play_human(q, r) {
                if self.sound_enabled {
                    self.audio.move_sound();
                }
                self.hover_cell = None;
                if self.sound_enabled && !was_game_over && self.game.game_over {
                    self.audio.game_over_sound();
                }
                self.schedule_computer_turn(cx, true);
            } else if self.sound_enabled && human_turn && !legal {
                self.audio.invalid_sound();
            }
            cx.notify();
        }
    }

    fn hover_board(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let next_hover = self.cell_at_position(event.position).filter(|(q, r)| {
            !self.game.game_over
                && self.game.current != self.game.computer
                && !self
                    .game
                    .flips_for_move(*q, *r, self.game.current)
                    .is_empty()
        });
        if self.hover_cell != next_hover {
            if self.sound_enabled && next_hover.is_some() {
                self.audio.hover_sound();
            }
            self.hover_cell = next_hover;
            cx.notify();
        }
    }

    fn cell_at_position(&self, position: Point<Pixels>) -> Option<(i32, i32)> {
        let geometry = self.geometry.borrow().clone()?;
        let hit_radius = geometry.hex_size * 0.92;
        geometry
            .cells
            .iter()
            .find(|cell| {
                let dx = f32::from(position.x - cell.center.x);
                let dy = f32::from(position.y - cell.center.y);
                (dx * dx + dy * dy).sqrt() <= hit_radius
            })
            .map(|cell| (cell.q, cell.r))
    }

    fn schedule_computer_turn(&mut self, cx: &mut Context<Self>, after_human_move: bool) {
        if self.game.game_over || self.game.current != self.game.computer {
            return;
        }

        self.turn_serial += 1;
        let turn_serial = self.turn_serial;
        let delay = random_computer_delay(after_human_move, self.speed_mode);
        cx.spawn(async move |this, cx| {
            let mut elapsed = Duration::ZERO;
            while elapsed < delay {
                let step = (delay - elapsed).min(Duration::from_millis(180));
                cx.background_executor().timer(step).await;
                elapsed += step;
                this.update(cx, |this, _| {
                    if this.turn_serial == turn_serial
                        && !this.game.game_over
                        && this.game.current == this.game.computer
                    {
                        if this.sound_enabled {
                            this.audio.selection_sound();
                        }
                    }
                })
                .ok();
            }

            this.update(cx, |this, cx| {
                if this.turn_serial == turn_serial {
                    let was_game_over = this.game.game_over;
                    if this.game.play_computer_move() {
                        if this.sound_enabled {
                            this.audio.computer_sound();
                        }
                        if this.sound_enabled && !was_game_over && this.game.game_over {
                            this.audio.game_over_sound();
                        }
                    }
                    if !this.game.game_over && this.game.current == this.game.computer {
                        this.schedule_computer_turn(cx, true);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle_speed_mode(&mut self, cx: &mut Context<Self>) {
        self.speed_mode = !self.speed_mode;
        cx.notify();
    }

    fn toggle_sound(&mut self, cx: &mut Context<Self>) {
        self.sound_enabled = !self.sound_enabled;
        cx.notify();
    }
}

impl Render for HexReversi {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = f32::from(window.viewport_size().width) < 860.0;
        let sidebar_width = if compact { px(176.0) } else { px(280.0) };
        let (black, white) = self.game.score();
        let black_role = if self.game.computer == Player::Black {
            "Computer"
        } else {
            "Human"
        };
        let white_role = if self.game.computer == Player::White {
            "Computer"
        } else {
            "Human"
        };
        let black_label = if compact {
            format!("Black: {black}")
        } else {
            format!("Black: {black} ({black_role})")
        };
        let white_label = if compact {
            format!("White: {white}")
        } else {
            format!("White: {white} ({white_role})")
        };
        let game_over = self.game.game_over;
        let speed_mode = self.speed_mode;
        let sound_enabled = self.sound_enabled;
        let right_status = self.game.status();
        let final_status = right_status.clone();
        let game = self.game.clone();
        let hover_cell = self.hover_cell;
        let geometry = self.geometry.clone();

        div()
            .size_full()
            .bg(rgb(0x11141b))
            .p_4()
            .flex()
            .gap_4()
            .text_color(rgb(0xeef3ff))
            .child(
                div()
                    .w(sidebar_width)
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .when(compact, |this| this.gap_3().p_3())
                    .when(!compact, |this| this.gap_5().p_5())
                    .rounded_xl()
                    .bg(rgb(0x1b2230))
                    .border_1()
                    .border_color(rgb(0x314058))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .pb_4()
                            .border_b_1()
                            .border_color(rgb(0x314058))
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Hexy Reversi"),
                            )
                            .when(!compact, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0xffd166))
                                        .child("build on top of gpui"),
                                )
                            })
                            .when(!compact, |this| {
                                this.child(
                                    div()
                                        .text_color(rgb(0x9ba8bf))
                                        .child("Trap stones in any of the directions."),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(0x202939))
                            .border_1()
                            .border_color(rgb(0x314058))
                            .when(!compact, |this| {
                                this.child(div().text_sm().text_color(rgb(0x9ba8bf)).child("Score"))
                            })
                            .child(
                                div()
                                    .text_xl()
                                    .when(compact, |this| this.flex().flex_col().gap_1())
                                    .child(black_label)
                                    .when(compact, |this| {
                                        this.child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(0x9ba8bf))
                                                .child(format!("({black_role})")),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(0xf6f2df))
                                    .when(compact, |this| this.flex().flex_col().gap_1())
                                    .child(white_label)
                                    .when(compact, |this| {
                                        this.child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(0x9ba8bf))
                                                .child(format!("({white_role})")),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(0x314058))
                            .when(!compact, |this| {
                                this.child(
                                    div().text_sm().text_color(rgb(0x9ba8bf)).child("Settings"),
                                )
                            })
                            .child(
                                div()
                                    .id("speed-mode")
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .hover(|style| style.opacity(0.85))
                                    .child("Speed mode")
                                    .child(
                                        div()
                                            .w(px(54.0))
                                            .h(px(30.0))
                                            .p(px(3.0))
                                            .rounded_full()
                                            .flex()
                                            .items_center()
                                            .when(speed_mode, |this| {
                                                this.justify_end().bg(rgb(0xffd166))
                                            })
                                            .when(!speed_mode, |this| {
                                                this.justify_start().bg(rgb(0x314058))
                                            })
                                            .child(
                                                div()
                                                    .size(px(24.0))
                                                    .rounded_full()
                                                    .bg(rgb(0xf6f2df)),
                                            ),
                                    )
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_speed_mode(cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .id("sound")
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .hover(|style| style.opacity(0.85))
                                    .child("Sound effects")
                                    .child(
                                        div()
                                            .w(px(54.0))
                                            .h(px(30.0))
                                            .p(px(3.0))
                                            .rounded_full()
                                            .flex()
                                            .items_center()
                                            .when(sound_enabled, |this| {
                                                this.justify_end().bg(rgb(0xffd166))
                                            })
                                            .when(!sound_enabled, |this| {
                                                this.justify_start().bg(rgb(0x314058))
                                            })
                                            .child(
                                                div()
                                                    .size(px(24.0))
                                                    .rounded_full()
                                                    .bg(rgb(0xf6f2df)),
                                            ),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sound(cx))),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div().w_full().flex().justify_center().child(
                            div()
                                .id("new-game")
                                .w(px(160.0))
                                .h(px(64.0))
                                .relative()
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.9))
                                .child(
                                    canvas(
                                        |_, _, _| {},
                                        |bounds, _, window, _| paint_hex_button(bounds, window),
                                    )
                                    .size_full(),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(rgb(0x1d1607))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("New game"),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .rounded_xl()
                    .bg(rgb(0x202939))
                    .border_1()
                    .border_color(rgb(0x314058))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p_4()
                            .border_b_1()
                            .border_color(rgb(0x314058))
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_color(rgb(0xffe4a3))
                                    .when(game_over, |this| this.opacity(0.0))
                                    .child(right_status),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            .child(
                                canvas(
                                    move |bounds, _, _| {
                                        let board_geometry = layout_board(bounds);
                                        *geometry.borrow_mut() = Some(board_geometry.clone());
                                        board_geometry
                                    },
                                    move |bounds, board_geometry, window, _| {
                                        paint_board(
                                            bounds,
                                            &board_geometry,
                                            &game,
                                            hover_cell,
                                            window,
                                        );
                                    },
                                )
                                .size_full(),
                            )
                            .when(game_over, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .p_4()
                                                .rounded_lg()
                                                .bg(rgb(0x2b3548))
                                                .border_1()
                                                .border_color(rgb(0xffd166))
                                                .text_color(rgb(0xffe4a3))
                                                .text_xl()
                                                .text_center()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(final_status),
                                        ),
                                )
                            }),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.click_board(event, cx)
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.hover_board(event, cx)
                    })),
            )
    }
}

fn random_player() -> Player {
    if random_index(2) == 0 {
        Player::Black
    } else {
        Player::White
    }
}

fn guide_color_for_score(score: i32) -> u32 {
    match score {
        0..=129 => 0x93c5fd,
        130..=229 => 0x5eead4,
        230..=329 => 0xffd166,
        _ => 0xfb7185,
    }
}

fn random_computer_delay(after_human_move: bool, speed_mode: bool) -> Duration {
    let (min, max) = if speed_mode {
        (SPEED_DELAY_MIN_MS, SPEED_DELAY_MAX_MS)
    } else if after_human_move {
        (COMPUTER_REPLY_DELAY_MIN_MS, COMPUTER_REPLY_DELAY_MAX_MS)
    } else {
        (COMPUTER_DELAY_MIN_MS, COMPUTER_DELAY_MAX_MS)
    };
    let span = max - min + 1;
    Duration::from_millis(min + random_index(span as usize) as u64)
}

fn random_index(len: usize) -> usize {
    if len == 0 {
        return 0;
    }

    (random_u64() as usize) % len
}

fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);

    // Mix the timestamp instead of using its low bit directly. Some platforms
    // report time in even nanosecond/microsecond steps, which made side choice
    // look stuck on one color.
    mix_u64(nanos ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

fn mix_u64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn is_on_board(q: i32, r: i32) -> bool {
    let s = -q - r;
    q.abs().max(r.abs()).max(s.abs()) <= RADIUS
}

fn cells() -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    for r in -RADIUS..=RADIUS {
        for q in -RADIUS..=RADIUS {
            if is_on_board(q, r) {
                result.push((q, r));
            }
        }
    }
    result
}

fn layout_board(bounds: Bounds<Pixels>) -> BoardGeometry {
    let board_units = (RADIUS * 2 + 1) as f32;
    let natural_width = HEX_SIZE * 3.0_f32.sqrt() * board_units;
    let natural_height = HEX_SIZE * 1.5 * board_units;
    let scale = (f32::from(bounds.size.width) / natural_width)
        .min(f32::from(bounds.size.height) / natural_height)
        .min(1.0);
    let hex_size = (HEX_SIZE * scale).max(12.0);
    let board_width = hex_size * 3.0_f32.sqrt() * board_units;
    let board_height = hex_size * 1.5 * board_units;
    let origin = point(
        bounds.origin.x + (bounds.size.width - px(board_width)) / 2.0,
        bounds.origin.y + (bounds.size.height - px(board_height)) / 2.0,
    );

    let cells = cells()
        .into_iter()
        .map(|(q, r)| {
            let x = hex_size * 3.0_f32.sqrt() * (q as f32 + r as f32 / 2.0);
            let y = hex_size * 1.5 * r as f32;
            CellGeometry {
                q,
                r,
                center: point(
                    origin.x + px(board_width / 2.0 + x),
                    origin.y + px(board_height / 2.0 + y),
                ),
            }
        })
        .collect();

    BoardGeometry { cells, hex_size }
}

fn paint_board(
    bounds: Bounds<Pixels>,
    geometry: &BoardGeometry,
    game: &Game,
    hover_cell: Option<(i32, i32)>,
    window: &mut Window,
) {
    window.paint_quad(gpui::fill(bounds, rgb(0x202939)));

    let hex_size = geometry.hex_size;
    let legal_moves = game.legal_moves(game.current);
    let legal: HashSet<(i32, i32)> = legal_moves.iter().copied().collect();
    let computer_thinking = !game.game_over && game.current == game.computer;
    let scored_moves = computer_thinking.then(|| game.scored_moves(game.current));
    let thinking_selection = computer_thinking
        .then(|| thinking_move_selection(scored_moves.as_deref().unwrap_or(&[])))
        .flatten();
    let active_flip = game.flip_animation.as_ref().and_then(|animation| {
        let progress = animation.start.elapsed().as_secs_f32() / FLIP_DURATION.as_secs_f32();
        (progress < 1.0).then_some((progress, animation))
    });
    if active_flip.is_some() || computer_thinking {
        window.request_animation_frame();
    }

    for cell in &geometry.cells {
        let alternate = (cell.q + cell.r + RADIUS) % 2 == 0;
        paint_hex(
            window,
            cell.center,
            hex_size,
            if alternate {
                rgb(0x2a724f).into()
            } else {
                rgb(0x2f8057).into()
            },
        );

        let is_legal = legal.contains(&(cell.q, cell.r));
        if hover_cell == Some((cell.q, cell.r)) && is_legal {
            paint_hex_overlay(
                window,
                cell.center,
                hex_size - 4.0,
                Into::<gpui::Hsla>::into(rgb(0xffd166)).alpha(0.24),
            );
        }

        if is_legal && !game.game_over {
            let guide_color = Into::<gpui::Hsla>::into(rgb(0xffd166));
            if let Some((_selected, color)) = thinking_selection
                .filter(|(selected, _)| selected.0 == cell.q && selected.1 == cell.r)
            {
                let (inner_radius, inner_alpha, outer_radius, outer_alpha) =
                    thinking_dot_pulse(hex_size);
                paint_circle(
                    window,
                    cell.center,
                    outer_radius * 1.15,
                    color.alpha(outer_alpha),
                );
                paint_circle(
                    window,
                    cell.center,
                    inner_radius * 1.35,
                    color.alpha(inner_alpha),
                );
            } else if computer_thinking {
                paint_circle(
                    window,
                    cell.center,
                    hex_size * 0.12,
                    guide_color.alpha(0.28),
                );
            } else {
                paint_circle(
                    window,
                    cell.center,
                    hex_size * 0.18,
                    guide_color.alpha(0.55),
                );
            }
        }

        if let Some(player) = game.board.get(&(cell.q, cell.r)) {
            if let Some((progress, animation)) = active_flip {
                if let Some(flip) = animation
                    .cells
                    .iter()
                    .find(|flip| flip.q == cell.q && flip.r == cell.r)
                {
                    paint_flip_piece(window, cell.center, progress, flip, hex_size);
                    continue;
                }
            }
            paint_circle(window, cell.center, hex_size * 0.48, player_color(*player));
        }
    }
}

fn thinking_move_selection(scored_moves: &[MoveScore]) -> Option<((i32, i32), gpui::Hsla)> {
    if scored_moves.is_empty() {
        return None;
    }

    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as usize / 180)
        .unwrap_or(0);
    let candidate_count = scored_moves.len().min(4);
    let selected = scored_moves[tick % candidate_count];
    let color = rgb(guide_color_for_score(selected.score)).into();

    Some(((selected.q, selected.r), color))
}

fn thinking_dot_pulse(hex_size: f32) -> (f32, f32, f32, f32) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f32)
        .unwrap_or(0.0);
    let wave = ((millis / 900.0) * std::f32::consts::TAU).sin() * 0.5 + 0.5;

    let inner_radius = hex_size * (0.18 + wave * 0.12);
    let outer_radius = hex_size * (0.36 + wave * 0.24);
    let inner_alpha = 0.55 + wave * 0.45;
    let outer_alpha = 0.38 * (1.0 - wave);

    (inner_radius, inner_alpha, outer_radius, outer_alpha)
}

fn player_color(player: Player) -> gpui::Hsla {
    match player {
        Player::Black => rgb(0x101218).into(),
        Player::White => rgb(0xf6f2df).into(),
    }
}

fn paint_flip_piece(
    window: &mut Window,
    center: Point<Pixels>,
    progress: f32,
    flip: &FlipCell,
    hex_size: f32,
) {
    let progress = progress.clamp(0.0, 1.0);
    let (player, width_factor) = if progress < 0.5 {
        (flip.from, 1.0 - progress * 2.0)
    } else {
        (flip.to, (progress - 0.5) * 2.0)
    };
    let width_factor = width_factor.max(0.08);

    paint_ellipse(
        window,
        center,
        hex_size * 0.48 * width_factor,
        hex_size * 0.48,
        player_color(player),
    );
}

fn paint_hex_overlay(window: &mut Window, center: Point<Pixels>, radius: f32, color: gpui::Hsla) {
    let mut fill = PathBuilder::fill();
    add_hex_path(&mut fill, center, radius);
    if let Ok(path) = fill.build() {
        window.paint_path(path, color);
    }
}

fn paint_hex_button(bounds: Bounds<Pixels>, window: &mut Window) {
    let center = bounds.center();
    let radius_x = f32::from(bounds.size.width) / 2.0;
    let radius_y = f32::from(bounds.size.height) / 2.0;

    let mut shadow = PathBuilder::fill();
    add_scaled_hex_path(
        &mut shadow,
        point(center.x, center.y + px(2.0)),
        radius_x,
        radius_y,
    );
    if let Ok(path) = shadow.build() {
        window.paint_path(path, Into::<gpui::Hsla>::into(rgb(0x000000)).alpha(0.18));
    }

    let mut fill = PathBuilder::fill();
    add_scaled_hex_path(&mut fill, center, radius_x - 2.0, radius_y - 2.0);
    if let Ok(path) = fill.build() {
        window.paint_path(path, rgb(0xffd166));
    }
}

fn paint_hex(window: &mut Window, center: Point<Pixels>, radius: f32, color: gpui::Hsla) {
    let mut border = PathBuilder::fill();
    add_hex_path(&mut border, center, radius + 1.8);
    if let Ok(path) = border.build() {
        window.paint_path(path, rgb(0x0b1712));
    }

    let mut fill = PathBuilder::fill();
    add_hex_path(&mut fill, center, radius - 1.5);
    if let Ok(path) = fill.build() {
        window.paint_path(path, color);
    }
}

fn add_hex_path(builder: &mut PathBuilder, center: Point<Pixels>, radius: f32) {
    add_scaled_hex_path(builder, center, radius, radius);
}

fn add_scaled_hex_path(
    builder: &mut PathBuilder,
    center: Point<Pixels>,
    radius_x: f32,
    radius_y: f32,
) {
    for i in 0..6 {
        let angle = (30.0 + i as f32 * 60.0).to_radians();
        let p = point(
            center.x + px(radius_x * angle.cos()),
            center.y + px(radius_y * angle.sin()),
        );
        if i == 0 {
            builder.move_to(p);
        } else {
            builder.line_to(p);
        }
    }
    builder.close();
}

fn paint_circle(window: &mut Window, center: Point<Pixels>, radius: f32, color: gpui::Hsla) {
    paint_ellipse(window, center, radius, radius, color);
}

fn paint_ellipse(
    window: &mut Window,
    center: Point<Pixels>,
    radius_x: f32,
    radius_y: f32,
    color: gpui::Hsla,
) {
    let radius_x = px(radius_x);
    let radius_y = px(radius_y);
    let mut builder = PathBuilder::fill();
    builder.move_to(point(center.x + radius_x, center.y));
    builder.arc_to(
        point(radius_x, radius_y),
        px(0.0),
        false,
        false,
        point(center.x - radius_x, center.y),
    );
    builder.arc_to(
        point(radius_x, radius_y),
        px(0.0),
        false,
        false,
        point(center.x + radius_x, center.y),
    );
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, gpui::size(px(980.0), px(700.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(gpui::size(px(680.0), px(500.0))),
                focus: true,
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Hexy Reversi");
                cx.new(|cx| {
                    let mut app = HexReversi::new();
                    app.schedule_computer_turn(cx, false);
                    app
                })
            },
        )
        .unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
