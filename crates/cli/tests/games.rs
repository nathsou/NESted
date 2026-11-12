//! Play the example cartridges through controller input.
use nested_compiler::{compile, Output, Options, Project};
use nested_runtime::Emulator;
use std::path::{Path, PathBuf};

pub const A: u8 = 1;
pub const B: u8 = 2;
pub const SELECT: u8 = 4;
pub const START: u8 = 8;
pub const UP: u8 = 16;
pub const DOWN: u8 = 32;
pub const LEFT: u8 = 64;
pub const RIGHT: u8 = 128;

pub struct Game {
    pub out: Output,
    pub emu: Emulator,
    pub frames: u64,
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn project(dir: &Path) -> Project {
    let mut p = Project {
        entry: "main.nst".into(),
        ..Default::default()
    };
    fn walk(dir: &Path, base: &Path, p: &mut Project) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, base, p);
                continue;
            }
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if rel.ends_with(".nst") {
                p.sources.insert(rel, std::fs::read_to_string(&path).unwrap());
            } else if rel.starts_with("assets/") {
                p.assets.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    walk(dir, dir, &mut p);
    p
}

impl Game {
    pub fn load(name: &str) -> Game {
        let out = compile(&project(&root().join("games").join(name)), Options::default()).unwrap_or_else(|e| panic!("{}", e.render()));
        let emu = Emulator::new(out.rom.clone(), 44100.0).unwrap();
        let mut g = Game { out, emu, frames: 0 };
        g.wait(30);
        g
    }
    pub fn frame(&mut self, buttons: u8) {
        self.emu.frame(buttons).unwrap();
        self.frames += 1;
        let dropped = self.byte("std::video::dropped");
        assert_eq!(dropped, 0, "background updates were dropped (frame {})", self.frames);
    }
    pub fn wait(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(0);
        }
    }
    /// Press for three frames (a quick human tap), then release.
    pub fn tap(&mut self, buttons: u8) {
        for _ in 0..3 {
            self.frame(buttons);
        }
        self.frame(0);
    }
    pub fn addr(&self, name: &str) -> u16 {
        self.out
            .linked
            .symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol {name}"))
            .address
    }
    pub fn byte(&mut self, name: &str) -> u8 {
        let a = self.addr(name);
        self.emu.read_ram(a)
    }
    pub fn word(&mut self, name: &str) -> u16 {
        let a = self.addr(name);
        self.emu.read_ram(a) as u16 | (self.emu.read_ram(a + 1) as u16) << 8
    }
    /// Wait until a byte has a value; panics after `limit` frames.
    pub fn until(&mut self, name: &str, value: u8, limit: usize) {
        for _ in 0..limit {
            if self.byte(name) == value {
                return;
            }
            self.frame(0);
        }
        panic!("{name} never became {value} (is {})", self.byte(name));
    }
    pub fn shot(&self, name: &str) {
        if let Ok(dir) = std::env::var("NESTED_SHOTS") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(Path::new(&dir).join(format!("{name}.png")), self.emu.png(2)).unwrap();
        }
    }
}

mod glide {
    use super::*;
    const PLAY: u8 = 1;
    const CLEAR: u8 = 5;

    fn solutions() -> Vec<String> {
        std::fs::read_to_string(root().join("games/glide/solutions.txt"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn play(g: &mut Game, moves: &str) {
        for m in moves.chars() {
            g.until("mode", PLAY, 120);
            let b = match m {
                'U' => UP,
                'D' => DOWN,
                'L' => LEFT,
                _ => RIGHT,
            };
            g.tap(b);
        }
    }

    #[test]
    fn every_level_is_solved_by_its_par_route() {
        let mut g = Game::load("glide");
        g.shot("glide-title");
        g.tap(START);
        g.until("mode", PLAY, 200);
        g.wait(10);
        let sols = solutions();
        assert_eq!(sols.len(), 25);
        for (level, sol) in sols.iter().enumerate() {
            assert_eq!(g.byte("level") as usize, level);
            if level == 18 {
                g.wait(20);
                g.shot("glide-level");
            }
            play(&mut g, sol);
            g.until("mode", CLEAR, 200);
            assert_eq!(g.word("moves") as usize, sol.len(), "level {} moves", level + 1);
            g.wait(70);
            if level == 0 {
                g.shot("glide-clear");
            }
            g.tap(A);
            if level < 24 {
                g.until("mode", PLAY, 200);
            }
        }
    }

    #[test]
    fn undo_restores_stars_and_moves() {
        let mut g = Game::load("glide");
        g.tap(START);
        g.until("mode", PLAY, 200);
        // Level 1: R collects the star.
        g.tap(RIGHT);
        g.until("mode", PLAY, 120);
        assert_eq!(g.byte("stars_left"), 0);
        assert_eq!(g.word("moves"), 1);
        g.tap(B);
        g.wait(5);
        assert_eq!(g.byte("stars_left"), 1);
        assert_eq!(g.word("moves"), 0);
    }
}

mod sprout {
    use super::*;
    const PLAY: u8 = 1;
    const CLEAR: u8 = 3;

    /// The level drawings from levels.nst.
    fn grid(level: usize) -> Vec<Vec<u8>> {
        let text = std::fs::read_to_string(root().join("games/sprout/levels.nst")).unwrap();
        let marker = format!("const TEXT{level}: [u8; _] = \"\n");
        let start = text.find(&marker).unwrap() + marker.len();
        text[start..].lines().take(13).map(|l| l.as_bytes().to_vec()).collect()
    }

    fn start(level: u8) -> Game {
        let mut g = Game::load("sprout");
        g.tap(START);
        g.until("mode", PLAY, 300);
        if level > 0 {
            // Jump straight to a later world.
            let addr = g.addr("level");
            g.emu.write_ram(addr, level);
        }
        g.wait(10);
        g
    }

    fn put(g: &mut Game, px: u16, py: u16) {
        let x = g.addr("x");
        let y = g.addr("y");
        let (x16, y16) = (px << 4, py << 4);
        g.emu.write_ram(x, x16 as u8);
        g.emu.write_ram(x + 1, (x16 >> 8) as u8);
        g.emu.write_ram(y, y16 as u8);
        g.emu.write_ram(y + 1, (y16 >> 8) as u8);
    }

    #[test]
    fn scrolls_collects_gems_and_reaches_the_first_flag() {
        let mut g = start(0);
        let map = grid(0);
        let solid = |c: usize, r: usize| r < 13 && c < map[0].len() && b"=#BSJ".contains(&map[r][c]);
        let mut jump = 0;
        let mut best = 0u16;
        for f in 0..3000 {
            if g.byte("mode") == CLEAR {
                break;
            }
            let px = g.word("x") >> 4;
            let py = g.word("y") >> 4;
            best = best.max(px);
            let col = (px as usize + 8) / 16;
            let row = (py as usize + 8) / 16;
            let ahead = col + 1;
            let grounded = g.byte("grounded") != 0;
            let blocked = solid(ahead, row) || solid(ahead, row.saturating_sub(1));
            let hole = |c: usize| !solid(c, row + 1) || map.get(row + 1).is_some_and(|r| r.get(c) == Some(&b'^'));
            // Jump at the last moment before a gap.
            let front = px as usize + 13;
            let gap = hole((front + 3) / 16) && !hole(front / 16);
            if grounded && jump == 0 && (blocked || gap || f % 97 == 0) {
                jump = 18;
            }
            let mut b = RIGHT;
            if jump > 0 {
                b |= A;
                jump -= 1;
            }
            let before = g.byte("mode");
            // Endless lives for the bot.
            let lives = g.addr("lives");
            g.emu.write_ram(lives, 9);
            g.frame(b);
            if before == PLAY && g.byte("mode") == 2 {
                eprintln!("bot died at x {px} y {py} (frame {f})");
            }
            if f == 400 {
                g.shot("sprout-play");
            }
        }
        assert!(g.byte("gems") > 0, "no gems collected");
        assert!(g.word("cam") > 256, "the camera did not scroll (x reached {best})");
        // Whatever the bot managed, the flag must finish the world.
        let flag_col = (0..map[0].len()).find(|c| (0..13).any(|r| map[r][*c] == b'F')).unwrap();
        let flag_row = (0..13).find(|r| map[*r][flag_col] == b'F').unwrap();
        if g.byte("mode") != CLEAR {
            g.until("mode", PLAY, 300);
            put(&mut g, flag_col as u16 * 16, flag_row as u16 * 16);
            g.wait(3);
        }
        assert_eq!(g.byte("mode"), CLEAR);
        g.wait(160);
        assert_eq!(g.byte("level"), 1, "the next world loads");
        g.until("mode", PLAY, 100);
        g.wait(30);
        g.shot("sprout-cavern");
        eprintln!("direct {} split {} {} {} cam {}", g.byte("std::video::direct"), g.byte("split_hi"), g.byte("split_x"), g.byte("split_lo"), g.word("cam"));
        g.wait(60);
        g.shot("sprout-cavern2");
    }

    #[test]
    fn slam_breaks_bricks_and_stomps() {
        let mut g = start(0);
        // The brick lid over the gem cache in the fourth segment (columns 50-57, row 10).
        put(&mut g, 52 * 16, 7 * 16);
        g.wait(2);
        g.tap(B);
        g.wait(30);
        let col = 52usize;
        let i = col * 13 + 10;
        let cleared = g.addr("cleared");
        let bits = g.emu.read_ram(cleared + (i >> 3) as u16);
        assert!(bits & (1 << (i & 7)) != 0, "the slam broke the brick below");
        g.shot("sprout-slam");
    }

    #[test]
    fn falling_costs_a_life_and_respawns() {
        let mut g = start(0);
        // The pit in the second segment (columns 20-21).
        put(&mut g, 20 * 16 + 4, 8 * 16);
        g.wait(200);
        assert_eq!(g.byte("lives"), 2);
        g.until("mode", PLAY, 100);
        g.wait(40);
        g.shot("sprout-respawn");
    }
}

#[test]
#[ignore]
fn sprout_trace() {
    let mut g = Game::load("sprout");
    g.tap(START);
    g.until("mode", 1, 300);
    let mut jump = 0;
    for f in 0..400 {
        let px = g.word("x") >> 4;
        let py = g.word("y") >> 4;
        let vy = g.word("vy") as i16;
        if f > 250 {
            println!("{f}: x {px} y {py} vy {vy} grounded {} pound {}", g.byte("grounded"), g.byte("pounding"));
        }
        if g.byte("grounded") != 0 && jump == 0 && f % 40 == 0 {
            jump = 18;
        }
        let mut b = RIGHT;
        if jump > 0 {
            b |= A;
            jump -= 1;
        }
        g.frame(b);
    }
}

#[test]
#[ignore]
fn sprout_irq_trace() {
    let mut g = Game::load("sprout");
    g.tap(START);
    g.until("mode", 1, 300);
    for world in 0..2u8 {
        if world == 1 {
            let lv = g.addr("level");
            // Teleport to the flag of world 1 and wait for world 2.
            let x = g.addr("x");
            g.emu.write_ram(x, ((105u16 * 16) << 4) as u8);
            g.emu.write_ram(x + 1, (((105u16 * 16) << 4) >> 8) as u8);
            let y = g.addr("y");
            g.emu.write_ram(y, ((32u16) << 4) as u8);
            g.emu.write_ram(y + 1, ((32u16 << 4) >> 8) as u8);
            g.wait(220);
            println!("level now {}", g.emu.read_ram(lv));
        }
        let irq = g.out.linked.symbols.iter().find(|s| s.name == "scanline@irq").map(|s| s.address).unwrap();
        for _ in 0..2 {
            g.emu.cpu.bus.ppu.frame_complete = false;
            while !g.emu.cpu.bus.ppu.frame_complete {
                if g.emu.cpu.pc == irq {
                    println!("world {world}: IRQ at scanline {} cycle {}", g.emu.cpu.bus.ppu.scanline, g.emu.cpu.bus.ppu.cycle);
                }
                g.emu.cpu.step();
            }
        }
    }
}

mod chess {
    use super::*;

    fn at(g: &mut Game, name: &str, i: u16) -> u8 {
        let a = g.addr(name);
        g.emu.read_ram(a + i)
    }

    fn tap(g: &mut Game, b: u8) {
        for _ in 0..3 {
            g.frame(b);
        }
        for _ in 0..3 {
            g.frame(0);
        }
    }

    #[test]
    fn plays_a_controller_move_and_the_engine_replies() {
        let mut g = Game::load("chess");
        // Menu: Black to NES level 1 (a fast reply), then start.
        tap(&mut g, UP);
        for _ in 0..4 {
            tap(&mut g, LEFT);
        }
        tap(&mut g, START);
        g.wait(30);
        assert_eq!(g.byte("screen_state"), 1, "game started");
        // The cursor starts on e2: select, two squares up, move.
        tap(&mut g, A);
        tap(&mut g, UP);
        tap(&mut g, UP);
        tap(&mut g, A);
        g.wait(240);
        assert_eq!(at(&mut g, "board", 0x34), 1, "white pawn on e4");
        assert_eq!(g.byte("game_plies"), 2, "1.e4 and the engine's reply");
        assert_eq!(g.byte("side"), 0, "white to move again");
        g.shot("chess");
    }

    #[test]
    fn ponders_and_stops_on_a_button() {
        let mut g = Game::load("chess");
        tap(&mut g, START);
        g.wait(30);
        let in_book = g.addr("in_book");
        g.emu.write_ram(in_book, 0);
        tap(&mut g, A);
        tap(&mut g, UP);
        tap(&mut g, UP);
        tap(&mut g, A);
        let mut frames = 0;
        while g.byte("game_plies") < 2 {
            g.frame(0);
            frames += 1;
            assert!(frames < 2000, "the engine replies");
        }
        let board = g.addr("board");
        let before: Vec<u8> = (0..128).map(|i| g.emu.read_ram(board + i)).collect();
        g.wait(400);
        assert_eq!(g.byte("pondering"), 1, "pondering while the human thinks");
        tap(&mut g, RIGHT);
        g.wait(30);
        assert_eq!(g.byte("pondering"), 0, "a button stops pondering");
        let after: Vec<u8> = (0..128).map(|i| g.emu.read_ram(board + i)).collect();
        let occupied = |i: usize| i & 8 == 0 || before[i & !8] != 0;
        let diff: Vec<usize> = (0..128).filter(|&i| occupied(i) && before[i] != after[i]).collect();
        assert!(diff.is_empty(), "pondering leaves the position as it was: {diff:?}");
    }

    #[test]
    fn bots_play_each_other() {
        let mut g = Game::load("chess");
        // White: human -> NES 1. Black: NES 5 -> NES 2.
        tap(&mut g, UP);
        tap(&mut g, UP);
        tap(&mut g, RIGHT);
        tap(&mut g, DOWN);
        for _ in 0..3 {
            tap(&mut g, LEFT);
        }
        assert_eq!(at(&mut g, "players", 0), 1);
        assert_eq!(at(&mut g, "players", 1), 2);
        tap(&mut g, START);
        let in_book = g.addr("in_book");
        g.emu.write_ram(in_book, 0);
        let mut frames = 0;
        while g.byte("game_plies") < 6 {
            g.frame(0);
            frames += 1;
            assert!(frames < 3000, "the bots keep moving without input");
        }
        assert_eq!(g.byte("screen_state"), 1, "still playing");
    }
}

#[test]
#[ignore]
fn chess_speed() {
    let mut g = Game::load("chess");
    // White: NES 5 (the default black level), black: human.
    for b in [UP, UP, RIGHT, RIGHT, RIGHT, RIGHT, DOWN] {
        for _ in 0..3 {
            g.frame(b);
        }
        for _ in 0..3 {
            g.frame(0);
        }
    }
    let p = g.addr("players");
    println!("players {} {}", g.emu.read_ram(p), g.emu.read_ram(p + 1));
    for _ in 0..3 {
        g.frame(START);
    }
    let in_book = g.addr("in_book");
    g.emu.write_ram(in_book, 0);
    g.wait(5);
    let mut best = 0u32;
    let mut frames = 0u32;
    while g.byte("game_plies") == 0 && frames < 1500 {
        g.frame(0);
        frames += 1;
        let n = g.word("nodes") as u32 | (g.word("nodes_high") as u32) << 16;
        best = best.max(n);
    }
    let (n0, n1) = (0u32, best * 10 / frames.max(1) * 6);
    println!("nodes in 10 s: {} ({} per second), plies {}", n1 - n0, (n1 - n0) / 10, g.byte("game_plies"));
}
