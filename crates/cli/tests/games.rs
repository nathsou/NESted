use nested_compiler::{compile, Assets, Compilation, Options};
use nested_runtime::Emulator;
use std::{collections::VecDeque, fs, path::PathBuf};
struct Game {
    c: Compilation,
    n: Emulator,
}
impl Game {
    fn load(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut assets = Assets::new();
        for entry in fs::read_dir(root.join("games/assets")).unwrap() {
            let path = entry.unwrap().path();
            assets.insert(
                format!("assets/{}", path.file_name().unwrap().to_str().unwrap()),
                fs::read(path).unwrap(),
            );
        }
        let c = compile(
            &fs::read_to_string(root.join(format!("games/{name}.nst"))).unwrap(),
            &assets,
            Options::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let n = Emulator::new(c.rom.clone(), 48000.0).unwrap();
        let mut g = Self { c, n };
        g.frames(0, 120);
        g
    }
    fn frames(&mut self, input: u8, count: usize) {
        for _ in 0..count {
            self.n.frame(input).unwrap();
        }
        assert_eq!(self.n.read_ram(0x37f), 0, "VRAM queue overflow");
    }
    fn tap(&mut self, input: u8) {
        self.frames(input, 3);
        self.frames(0, 3);
    }
    fn byte(&mut self, name: &str) -> u8 {
        self.at(name, 0)
    }
    fn at(&mut self, name: &str, index: u16) -> u8 {
        self.n
            .read_ram(self.c.assembly.symbols[&format!("__v_{name}")] + index)
    }
    fn set(&mut self, name: &str, index: u16, value: u8) {
        let address = self.c.assembly.symbols[&format!("__v_{name}")] + index;
        self.n.write_ram(address, value);
    }
}
#[test]
fn nonogram_solves_with_controller_and_undo() {
    let mut g = Game::load("bloom");
    g.tap(1);
    assert_eq!(g.at("cells", 0), 1);
    g.tap(4);
    assert_eq!(g.at("cells", 0), 0);
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../games/assets/bloom.puzzles.bin");
    let solution = fs::read(root).unwrap();
    for y in 0..8 {
        for step in 0..8 {
            let x = if y % 2 == 0 { step } else { 7 - step };
            if solution[y * 8 + x] != 0 {
                g.tap(1);
            }
            if step < 7 {
                g.tap(if y % 2 == 0 { 128 } else { 64 });
            }
        }
        if y < 7 {
            g.tap(32);
        }
    }
    assert_eq!(g.byte("solved"), 1);
    g.tap(8);
    assert_eq!(g.byte("level"), 1);
    assert_eq!(g.byte("solved"), 0);
}
#[test]
fn rhythm_chart_can_be_completed_without_misses() {
    for difficulty in 0..3 {
        let mut g = Game::load("starstring");
        for _ in 0..((difficulty + 2) % 3) {
            g.tap(4);
        }
        assert_eq!(g.byte("difficulty"), difficulty);
        g.tap(8);
        let mut last = 0u8;
        for _ in 0..4300 {
            if g.byte("mode") == 2 {
                break;
            }
            let mut input = 0;
            for i in 0..12 {
                let y = g.at("note_y", i);
                if (188..=190).contains(&y) {
                    input |= g.at("note_mask", i);
                }
                if g.at("sustain", i) > 0 {
                    input |= g.at("sustain_mask", i);
                }
            } // Avoid held-lane edge suppression for consecutive notes.
            if input == last && input != 0 {
                let mut sustained = 0;
                for i in 0..12 {
                    if g.at("sustain", i) > 0 {
                        sustained |= g.at("sustain_mask", i);
                    }
                }
                input = sustained;
            }
            let pad =
                ((input & 1) << 6) | ((input & 2) << 4) | ((input & 4) << 2) | ((input & 8) << 4);
            g.frames(pad, 1);
            last = input;
        }
        assert_eq!(g.byte("chart_step"), 192);
        assert_eq!(g.byte("mode"), 2);
        assert_eq!(g.byte("misses"), 0);
        assert!(g.byte("perfect") > 100);
    }
}
#[test]
fn platformer_jump_dash_death_and_restart() {
    let mut g = Game::load("skythread");
    let ground = g.byte("player_y");
    g.frames(1, 8);
    assert!(g.byte("player_y") < ground - 10);
    g.frames(0, 2);
    g.frames(130, 3);
    assert!(g.byte("dash_time") > 0);
    assert_eq!(g.byte("can_dash"), 0);
    g.frames(128, 90);
    g.tap(8);
    assert_eq!(g.byte("room"), 0);
    assert_eq!(g.byte("deaths"), 0);
    assert_eq!(g.byte("finished"), 0);
}
#[test]
fn dungeon_is_connected_and_keys_advance_all_six_floors() {
    let mut g = Game::load("emberkeep");
    for floor in 0..6 {
        assert_eq!(g.byte("floor"), floor);
        let cells = (0..192).map(|i| g.at("dungeon", i)).collect::<Vec<_>>();
        let start = (g.byte("player_y") as usize) * 16 + g.byte("player_x") as usize;
        let mut seen = [false; 192];
        let mut q = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = q.pop_front() {
            for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let x = (i % 16) as i32 + dx;
                let y = (i / 16) as i32 + dy;
                if (0..16).contains(&x) && (0..12).contains(&y) {
                    let j = y as usize * 16 + x as usize;
                    if cells[j] != 0 && !seen[j] {
                        seen[j] = true;
                        q.push_back(j);
                    }
                }
            }
        }
        for (i, c) in cells.iter().enumerate() {
            if *c != 0 {
                assert!(seen[i]);
            }
        }
        // A targeted map fixture tests transitions separately from controller playability.
        for e in 0..6 {
            g.set("enemy_hp", e, 0);
        }
        let stair = cells.iter().position(|v| *v == 2).unwrap();
        let x = stair % 16;
        let y = stair / 16;
        g.set("player_x", 0, (x - 1) as u8);
        g.set("player_y", 0, y as u8);
        g.set("dungeon", (stair - 1) as u16, 1);
        g.set("key_found", 0, 1);
        g.tap(128);
        g.frames(0, 100);
    }
    assert_eq!(g.byte("victory"), 1);
    g.tap(8);
    g.frames(0, 80);
    assert_eq!(g.byte("floor"), 0);
    assert_eq!(g.byte("over"), 0);
}

#[test]
fn dungeon_controller_can_win_a_complete_run() {
    let mut g = Game::load("emberkeep");
    for _ in 0..500 {
        if g.byte("over") != 0 {
            break;
        }
        if g.byte("health") <= 6 && g.byte("potions") > 0 {
            g.tap(2);
            continue;
        }
        let cells = (0..192).map(|i| g.at("dungeon", i)).collect::<Vec<_>>();
        let target = if g.byte("key_found") == 0 { 3 } else { 2 };
        let goal = cells.iter().position(|c| *c == target).unwrap();
        let start = g.byte("player_y") as usize * 16 + g.byte("player_x") as usize;
        let mut parent = vec![None; 192];
        let mut q = VecDeque::from([start]);
        parent[start] = Some((start, 0));
        while let Some(i) = q.pop_front() {
            if i == goal {
                break;
            }
            for (dx, dy, pad) in [(0, 1, 32), (0, -1, 16), (1, 0, 128), (-1, 0, 64)] {
                let x = (i % 16) as i32 + dx;
                let y = (i / 16) as i32 + dy;
                if (0..16).contains(&x) && (0..12).contains(&y) {
                    let j = y as usize * 16 + x as usize;
                    if cells[j] != 0 && parent[j].is_none() {
                        parent[j] = Some((i, pad));
                        q.push_back(j);
                    }
                }
            }
        }
        let mut next = goal;
        while parent[next].unwrap().0 != start {
            next = parent[next].unwrap().0;
        }
        let before = g.byte("floor");
        g.tap(parent[next].unwrap().1);
        if before != g.byte("floor") {
            g.frames(0, 100);
        }
    }
    assert_eq!(
        g.byte("victory"),
        1,
        "controller navigation and combat should finish all floors"
    );
}

#[test]
fn platformer_authored_rooms_have_controller_routes() {
    let mut g = Game::load("skythread");
    let routes: &[&[(u8, u8)]] = &[
        &[(64, 177), (112, 145), (160, 113), (192, 81), (208, 49)],
        &[(56, 129), (104, 97), (152, 65), (216, 33)],
        &[(56, 129), (88, 97), (144, 49), (208, 33)],
        &[(40, 129), (80, 97), (136, 65), (208, 33)],
        &[(88, 129), (56, 97), (136, 65), (208, 33)],
        &[(40, 129), (64, 97), (136, 65), (224, 33)],
    ];
    for (room, route) in routes.iter().enumerate() {
        g.frames(0, 60);
        if room == 3 || room == 5 {
            g.frames(64, 12);
            g.frames(0, 3);
        }
        for &(goal_x, goal_y) in *route {
            let mut last = 0;
            for _ in 0..240 {
                if g.byte("room") as usize != room || g.byte("finished") != 0 {
                    break;
                }
                let x = g.byte("player_x");
                let y = g.byte("player_y");
                let ground = g.byte("grounded") != 0;
                if x.abs_diff(goal_x) <= 3 && y.abs_diff(goal_y) <= 2 && ground {
                    break;
                }
                // Rise beside the upper ledge before moving over it.
                let mut pad = if y > goal_y.saturating_add(10) {
                    0
                } else if x + 2 < goal_x {
                    128
                } else if x > goal_x + 2 {
                    64
                } else {
                    0
                };
                if !ground || last & 1 == 0 || g.byte("velocity_y") >= 128 {
                    pad |= 1;
                }
                if !ground
                    && g.byte("velocity_y") < 128
                    && g.byte("can_dash") != 0
                    && y >= goal_y.saturating_sub(15)
                {
                    pad = 19;
                } else if !ground
                    && g.byte("velocity_y") < 128
                    && g.byte("can_dash") != 0
                    && x.abs_diff(goal_x) > 32
                {
                    pad = 3 | if x < goal_x { 128 } else { 64 };
                }
                if room == 2 && goal_x == 144 && x >= 120 && y > 52 {
                    pad |= 128;
                    if !ground && g.byte("velocity_y") < 128 && last & 1 != 0 {
                        pad &= !1;
                    }
                }
                g.frames(pad, 1);
                last = pad;
            }
            g.frames(0, 2);
        }
        // Walk across the exit tile after landing on its platform.
        for _ in 0..60 {
            if g.byte("room") as usize != room || g.byte("finished") != 0 {
                break;
            }
            g.frames(128, 1);
        }
        assert_eq!(
            g.byte("room") as usize,
            room + 1,
            "room {room}, x={}, y={}, deaths={}",
            g.byte("player_x"),
            g.byte("player_y"),
            g.byte("deaths")
        );
    }
    assert_eq!(g.byte("finished"), 1);
}
