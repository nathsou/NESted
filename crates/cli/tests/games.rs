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
