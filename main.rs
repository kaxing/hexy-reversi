use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Application, Bounds, Context, MouseDownEvent, PathBuilder, Pixels, Point, Render, Window,
    WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, rgb,
};

const RADIUS: i32 = 4;
const HEX_SIZE: f32 = 34.0;
const DIRECTIONS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
const FLIP_DURATION: Duration = Duration::from_millis(420);
const COMPUTER_DELAY_MIN_MS: u64 = 550;
const COMPUTER_DELAY_MAX_MS: u64 = 1_300;

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

    fn run_computer_turn(&mut self) {
        while !self.game_over && self.current == self.computer {
            let moves = self.legal_moves(self.current);
            if moves.is_empty() {
                self.advance_past_passes();
                continue;
            }

            let (q, r) = moves[random_index(moves.len())];
            let flips = self.flips_for_move(q, r, self.current);
            self.apply_move(q, r, flips);
            if !self.game_over {
                self.message = Some("Computer played.".to_string());
            }
        }
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
                std::cmp::Ordering::Greater => format!("Game over: Black wins, {black}-{white}."),
                std::cmp::Ordering::Less => format!("Game over: White wins, {black}-{white}."),
                std::cmp::Ordering::Equal => format!("Game over: draw, {black}-{white}."),
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
    turn_serial: u64,
}

impl HexReversi {
    fn new() -> Self {
        Self {
            game: Game::new(),
            geometry: Rc::new(RefCell::new(None)),
            turn_serial: 0,
        }
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.game = Game::new();
        self.turn_serial += 1;
        self.schedule_computer_turn(cx);
        cx.notify();
    }

    fn click_board(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry.borrow().clone() else {
            return;
        };

        let hit_radius = HEX_SIZE * 0.92;
        if let Some(cell) = geometry.cells.iter().find(|cell| {
            let dx = f32::from(event.position.x - cell.center.x);
            let dy = f32::from(event.position.y - cell.center.y);
            (dx * dx + dy * dy).sqrt() <= hit_radius
        }) {
            if self.game.play_human(cell.q, cell.r) {
                self.schedule_computer_turn(cx);
            }
            cx.notify();
        }
    }

    fn schedule_computer_turn(&mut self, cx: &mut Context<Self>) {
        if self.game.game_over || self.game.current != self.game.computer {
            return;
        }

        self.turn_serial += 1;
        let turn_serial = self.turn_serial;
        let delay = random_computer_delay();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                if this.turn_serial == turn_serial {
                    this.game.run_computer_turn();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

impl Render for HexReversi {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let game_over = self.game.game_over;
        let right_status = self.game.status();
        let final_status = right_status.clone();
        let game = self.game.clone();
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
                    .w(px(280.0))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .rounded_xl()
                    .bg(rgb(0x1b2230))
                    .border_1()
                    .border_color(rgb(0x314058))
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Hexy Reversi"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xffd166))
                            .child("made with gpui"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x9ba8bf))
                            .child("Trap stones in any of the 6 straight hex directions."),
                    )
                    .child(
                        div()
                            .text_xl()
                            .child(format!("Black: {black} ({black_role})")),
                    )
                    .child(
                        div()
                            .text_xl()
                            .text_color(rgb(0xf6f2df))
                            .child(format!("White: {white} ({white_role})")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("new-game")
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(0xffd166))
                            .text_color(rgb(0x1d1607))
                            .font_weight(gpui::FontWeight::BOLD)
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .child("New game")
                            .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
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
                                        paint_board(bounds, &board_geometry, &game, window);
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
                    ),
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

fn random_computer_delay() -> Duration {
    let span = COMPUTER_DELAY_MAX_MS - COMPUTER_DELAY_MIN_MS + 1;
    Duration::from_millis(COMPUTER_DELAY_MIN_MS + random_index(span as usize) as u64)
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
    let mut x = nanos ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15);
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
    let board_width = HEX_SIZE * 3.0_f32.sqrt() * ((RADIUS * 2 + 1) as f32);
    let board_height = HEX_SIZE * 1.5 * ((RADIUS * 2 + 1) as f32);
    let origin = point(
        bounds.origin.x + (bounds.size.width - px(board_width)) / 2.0,
        bounds.origin.y + (bounds.size.height - px(board_height)) / 2.0,
    );

    let cells = cells()
        .into_iter()
        .map(|(q, r)| {
            let x = HEX_SIZE * 3.0_f32.sqrt() * (q as f32 + r as f32 / 2.0);
            let y = HEX_SIZE * 1.5 * r as f32;
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

    BoardGeometry { cells }
}

fn paint_board(bounds: Bounds<Pixels>, geometry: &BoardGeometry, game: &Game, window: &mut Window) {
    window.paint_quad(gpui::fill(bounds, rgb(0x202939)));

    let legal: HashSet<(i32, i32)> = game.legal_moves(game.current).into_iter().collect();
    let active_flip = game.flip_animation.as_ref().and_then(|animation| {
        let progress = animation.start.elapsed().as_secs_f32() / FLIP_DURATION.as_secs_f32();
        (progress < 1.0).then_some((progress, animation))
    });
    if active_flip.is_some() {
        window.request_animation_frame();
    }

    for cell in &geometry.cells {
        let alternate = (cell.q + cell.r + RADIUS) % 2 == 0;
        paint_hex(
            window,
            cell.center,
            HEX_SIZE,
            if alternate {
                rgb(0x2a724f).into()
            } else {
                rgb(0x2f8057).into()
            },
        );

        if legal.contains(&(cell.q, cell.r)) && !game.game_over {
            paint_circle(
                window,
                cell.center,
                HEX_SIZE * 0.18,
                Into::<gpui::Hsla>::into(rgb(0xffd166)).alpha(0.55),
            );
        }

        if let Some(player) = game.board.get(&(cell.q, cell.r)) {
            if let Some((progress, animation)) = active_flip {
                if let Some(flip) = animation
                    .cells
                    .iter()
                    .find(|flip| flip.q == cell.q && flip.r == cell.r)
                {
                    paint_flip_piece(window, cell.center, progress, flip);
                    continue;
                }
            }
            paint_circle(window, cell.center, HEX_SIZE * 0.48, player_color(*player));
        }
    }
}

fn player_color(player: Player) -> gpui::Hsla {
    match player {
        Player::Black => rgb(0x101218).into(),
        Player::White => rgb(0xf6f2df).into(),
    }
}

fn paint_flip_piece(window: &mut Window, center: Point<Pixels>, progress: f32, flip: &FlipCell) {
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
        HEX_SIZE * 0.48 * width_factor,
        HEX_SIZE * 0.48,
        player_color(player),
    );
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
    for i in 0..6 {
        let angle = (30.0 + i as f32 * 60.0).to_radians();
        let p = point(
            center.x + px(radius * angle.cos()),
            center.y + px(radius * angle.sin()),
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
                focus: true,
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut app = HexReversi::new();
                    app.schedule_computer_turn(cx);
                    app
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
