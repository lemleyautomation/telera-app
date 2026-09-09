//! Layout stress test — a fake trading terminal ("NOCTURNE") built entirely
//! from the markdown layout system (`examples/layouts/Dashboard.md`), tuned to
//! sit just under clay's default 8192-element ceiling.
//!
//! The whole dataset (~1000 candles, ~90 watchlist rows, a 68-level order book,
//! a trade tape) is regenerated every frame and the window renders
//! continuously, so this exercises the full path at near-max element count
//! every frame: clay layout + the render-command fingerprint + lyon
//! tessellation + composite. Expect real CPU load — that's the point.
//!
//! Wants a wide window (the default is 1760×1000; a tiling WM may shrink it).
//! Run: `cargo run --release --example stress`

use std::time::{Duration, Instant};
use telera_app::*;

// --- element-count knobs -----------------------------------------------------
// `element_estimate()` mirrors Dashboard.md's structure. Going over clay's 8192
// default doesn't print anything - the whole layout is replaced by a red
// "Layout elements exceeded" message - so the count here is set to land at
// ~8100 (≈ 99% of the cap) and left there.
/// Target render period: `set_viewport_frame_interval` paces the redraw loop to
/// this, and `update` regenerates the dataset + ticks the fps counter on the
/// same cadence.
const FRAME: f64 = 1.0 / 144.0;

const PILLS: usize = 12;
const CANDLES: usize = 1010;
const WATCH: usize = 88;
const LEVELS: usize = 34;
const TAPE: usize = 54;
const PRICE_AXIS: usize = 14;
const TIME_AXIS: usize = 14;

fn element_estimate() -> usize {
    PILLS * 4          // header index pills (row + 3 texts)
        + WATCH * 11   // watchlist rows (row + 5 wrapped cells)
        + CANDLES * 6  // price + volume bars (col + gap + body, two panes)
        + PRICE_AXIS
        + TIME_AXIS * 2 // x-axis labels (cell + text)
        + LEVELS * 14  // bid + ask rows (row + 3 wrapped cells)
        + TAPE * 8     // tape rows (row + dot + 3 wrapped cells)
        + 48           // panels, headers, scroll wrappers, chrome
}

// --- palette ---------------------------------------------------------------
fn up() -> Color {
    Color::rgb(38, 166, 105)
}
fn down() -> Color {
    Color::rgb(224, 41, 46)
}
fn dim_up() -> Color {
    Color::rgb(24, 71, 52)
}
fn dim_down() -> Color {
    Color::rgb(84, 28, 32)
}

// --- one row of each list ------------------------------------------------------

#[derive(FieldAccess, Default, Clone)]
struct Pill {
    idx_sym: String,
    idx_val: String,
    idx_chg: String,
    idx_color: Color,
}

#[derive(FieldAccess, Default, Clone)]
struct Candle {
    /// % offset from the top of the price pane where the body starts.
    cd_top: f32,
    /// % height of the body.
    cd_range: f32,
    cd_color: Color,
    /// same, for the volume pane.
    vol_top: f32,
    vol_range: f32,
    vol_color: Color,
}

#[derive(FieldAccess, Default, Clone)]
struct Ticker {
    sym: String,
    last: String,
    chg: String,
    chg_pct: String,
    spark: String,
    chg_color: Color,
}

#[derive(FieldAccess, Default, Clone)]
struct Level {
    price: String,
    size: String,
    cum: String,
    /// Row tint - brighter the deeper into the book, so the panel reads as a
    /// depth ladder without needing overlapping bars.
    side_color: Color,
}

#[derive(FieldAccess, Default, Clone)]
struct Trade {
    t_time: String,
    t_px: String,
    t_sz: String,
    dot_color: Color,
}

#[derive(LayoutRunnerReflection, Default)]
struct Nocturne {
    // header / chart header text
    clock: String,
    hud: String,
    sym_big: String,
    last_big: String,
    last_color: Color,
    o: String,
    h: String,
    l: String,
    c: String,
    spread: String,

    // lists
    indices: Vec<Pill>,
    candles: Vec<Candle>,
    watchlist: Vec<Ticker>,
    asks: Vec<Level>,
    bids: Vec<Level>,
    tape: Vec<Trade>,
    price_axis: Vec<String>,
    time_axis: Vec<String>,

    // --- sim state (not referenced by the layout) ---
    rng: u64,
    frame_t: Option<Instant>,
    fps: f32,
    frames: u64,
    px: f32,
}

// no clickable widgets in this example - the layout only reads data
impl LayoutReflector for Nocturne {}

// tiny xorshift so the example has no rng dependency
fn xs(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}
fn unit(state: &mut u64) -> f32 {
    (xs(state) >> 40) as f32 / (1u64 << 24) as f32
}
fn spread(state: &mut u64) -> f32 {
    unit(state) - 0.5
}

const NAMES: [&str; 30] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "META", "GOOGL", "TSLA", "AVGO", "AMD", "NFLX", "CRM", "ADBE",
    "ORCL", "INTC", "QCOM", "TXN", "MU", "PANW", "SNOW", "PLTR", "SHOP", "UBER", "ABNB", "COIN",
    "SQ", "PYPL", "SOFI", "RIVN", "DKNG", "ARM",
];

impl Nocturne {
    fn regen(&mut self) {
        let r = &mut self.rng;

        // ---- price walk -> candles ------------------------------------------
        let mut path = Vec::with_capacity(CANDLES + 1);
        let mut p = self.px;
        for _ in 0..=CANDLES {
            p += spread(r) * 1.4 + (200.0 - p) * 0.002; // drift back toward 200
            path.push(p);
        }
        self.px = *path.last().unwrap();

        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &v in &path {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        let span = (hi - lo).max(0.01);

        // clay's SIZING_PERCENT wants a 0.0..=1.0 fraction.
        self.candles.clear();
        let mut vprev = 0.6f32;
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (top, bot) = (a.max(b), a.min(b));
            let cd_top = ((hi - top) / span).clamp(0.0, 0.98);
            let cd_range = ((top - bot) / span).max(0.006).min(1.0 - cd_top);
            let rising = b >= a;

            vprev = (vprev * 0.85 + unit(r) * 0.6).clamp(0.04, 1.0);

            self.candles.push(Candle {
                cd_top,
                cd_range,
                cd_color: if rising { up() } else { down() },
                vol_top: 1.0 - vprev,
                vol_range: vprev,
                vol_color: if rising { dim_up() } else { dim_down() },
            });
        }

        // chart header
        let last = self.px;
        let first = path[path.len().saturating_sub(120).min(path.len() - 1)];
        let chg = last - first;
        self.sym_big = "AAPL".into();
        self.last_big = format!("{last:>8.2}");
        self.last_color = if chg >= 0.0 { up() } else { down() };
        self.o = format!("O {:.2}", first);
        self.h = format!("H {:.2}", hi);
        self.l = format!("L {:.2}", lo);
        self.c = format!("C {:.2}  {:+.2} ({:+.2}%)", last, chg, chg / first * 100.0);

        // ---- price axis / time axis ---------------------------------------
        self.price_axis.clear();
        for i in 0..PRICE_AXIS {
            let v = hi - span * i as f32 / (PRICE_AXIS - 1) as f32;
            self.price_axis.push(format!("{v:.2}"));
        }
        self.time_axis.clear();
        for i in 0..TIME_AXIS {
            let m = 9 * 60 + 30 + (390 * i / (TIME_AXIS - 1));
            self.time_axis.push(format!("{:02}:{:02}", m / 60, m % 60));
        }

        // ---- watchlist --------------------------------------------------------
        self.watchlist.clear();
        for i in 0..WATCH {
            let base = 20.0 + (i as f32 * 7.3) % 480.0;
            let ch = spread(r) * base * 0.05;
            let mut mini = base;
            let mut spark = String::with_capacity(10);
            for _ in 0..10 {
                mini += spread(r) * base * 0.02;
                let bar = ((mini - base) / (base * 0.05) + 0.5).clamp(0.0, 0.999);
                spark.push(" ▁▂▃▄▅▆▇█".chars().nth((bar * 8.0) as usize + 1).unwrap());
            }
            self.watchlist.push(Ticker {
                sym: NAMES[i % NAMES.len()].into(),
                last: format!("{:.2}", base + ch),
                chg: format!("{ch:+.2}"),
                chg_pct: format!("{:+.2}%", ch / base * 100.0),
                spark,
                chg_color: if ch >= 0.0 { up() } else { down() },
            });
        }

        // ---- order book ----------------------------------------------------
        let mid = self.px;
        let tick = 0.01;
        self.asks.clear();
        self.bids.clear();
        let mut cum_a = 0.0f32;
        let mut cum_b = 0.0f32;
        let mut rows_a = Vec::with_capacity(LEVELS);
        let mut rows_b = Vec::with_capacity(LEVELS);
        for i in 0..LEVELS {
            let sz_a = 20.0 + unit(r) * 900.0;
            let sz_b = 20.0 + unit(r) * 900.0;
            cum_a += sz_a;
            cum_b += sz_b;
            rows_a.push((mid + tick * (i as f32 + 1.0), sz_a, cum_a));
            rows_b.push((mid - tick * (i as f32 + 1.0), sz_b, cum_b));
        }
        let max_cum = cum_a.max(cum_b).max(1.0);
        // encode cumulative depth as the row tint (deeper into the book = brighter)
        for &(price, sz, cum) in rows_a.iter().rev() {
            let d = (cum / max_cum * 46.0) as u8;
            self.asks.push(Level {
                price: format!("{price:.2}"),
                size: format!("{sz:.0}"),
                cum: format!("{cum:.0}"),
                side_color: Color::rgb(24 + d, 11, 13),
            });
        }
        for &(price, sz, cum) in &rows_b {
            let d = (cum / max_cum * 46.0) as u8;
            self.bids.push(Level {
                price: format!("{price:.2}"),
                size: format!("{sz:.0}"),
                cum: format!("{cum:.0}"),
                side_color: Color::rgb(10, 24 + d, 15),
            });
        }
        self.spread = format!("SPREAD  {:.2}  ({:.1} bps)", tick, tick / mid * 1e4);

        // ---- tape ---------------------------------------------------------
        self.tape.clear();
        for i in 0..TAPE {
            let aggressive_buy = unit(r) > 0.5;
            let m = 16 * 60 - i;
            self.tape.push(Trade {
                t_time: format!("{:02}:{:02}:{:02}", m / 60 % 24, m % 60, (xs(r) % 60) as u32),
                t_px: format!("{:.2}", mid + spread(r) * 0.1),
                t_sz: format!("{}", 1 + (xs(r) % 400)),
                dot_color: if aggressive_buy { up() } else { down() },
            });
        }

        // ---- header index strip -----------------------------------------------
        const IDX: [(&str, f32); 12] = [
            ("SPX", 5100.0),
            ("NDX", 17800.0),
            ("DJI", 38900.0),
            ("RUT", 2050.0),
            ("VIX", 13.4),
            ("DXY", 104.2),
            ("UST10", 4.28),
            ("GOLD", 2340.0),
            ("WTI", 78.5),
            ("BTC", 64200.0),
            ("ETH", 3350.0),
            ("EURUSD", 1.083),
        ];
        self.indices.clear();
        for (sym, base) in IDX {
            let pct = spread(r) * 2.4;
            self.indices.push(Pill {
                idx_sym: sym.into(),
                idx_val: format!("{base:.2}"),
                idx_chg: format!("{pct:+.2}%"),
                idx_color: if pct >= 0.0 { up() } else { down() },
            });
        }
    }
}

impl App for Nocturne {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes()
                .with_title("NOCTURNE")
                .with_inner_size(LogicalSize::new(1760, 1000)),
            window_name: "Dashboard".to_string(),
            page: None,
            watch_path: RunType::Watch("examples/layouts".to_string()),
        }
    }

    fn onload(&mut self, api: &mut API) {
        self.rng = 0x9E3779B97F4A7C15;
        self.px = 198.0;
        self.regen();
        // hammer it: rebuild + re-render every frame.
        api.set_viewport_continuous("Dashboard", true);
        api.set_viewport_frame_interval("Dashboard", Duration::from_secs_f64(FRAME));
    }

    fn update(&mut self, _viewport: Option<&str>, _api: &mut API) {
        // `update` runs once per event-loop wake, which (Wayland frame callback +
        // the pacing timer) is a few times per rendered frame. Only rebuild the
        // ~8k data points (and tick the fps counter) at roughly the render
        // cadence - same period as the frame interval above.
        let now = Instant::now();
        let dt = self.frame_t.map(|p| now.duration_since(p).as_secs_f32());
        if matches!(dt, Some(d) if (d as f64) < FRAME * 0.9) {
            return;
        }
        if let Some(d) = dt {
            self.fps = self.fps * 0.9 + (1.0 / d.max(1e-4)) * 0.1;
        }
        self.frame_t = Some(now);
        self.frames += 1;

        self.regen();

        self.clock = format!("{:02}:{:02}:{:02} EDT", 16, (self.frames / 60) % 60, self.frames % 60);
        self.hud = format!(
            "~{} elements  /  8192 cap    ·    {:.0} fps    ·    frame {}",
            element_estimate(),
            self.fps,
            self.frames
        );
    }
}

fn main() {
    run::<Nocturne>(Nocturne::default());
}
