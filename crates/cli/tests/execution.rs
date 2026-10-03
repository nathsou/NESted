use nested_compiler::{compile, Assets, Options};
use nested_runtime::Emulator;
fn run(source: &str, optimize: bool, frames: usize) -> (nested_compiler::Compilation, Emulator) {
    let c =
        compile(source, &Assets::new(), Options { optimize }).unwrap_or_else(|e| panic!("{e:?}"));
    let mut n = Emulator::new(c.rom.clone(), 44100.0).unwrap();
    for _ in 0..frames {
        n.frame(0).unwrap();
    }
    (c, n)
}
fn read(c: &nested_compiler::Compilation, n: &mut Emulator, name: &str) -> u16 {
    let address = c.assembly.symbols[&format!("__v_{name}")];
    u16::from(n.read_ram(address)) | u16::from(n.read_ram(address + 1)) << 8
}
#[test]
fn arithmetic_and_calls_execute() {
    let source = r#"
var a: u8 = 0; var b: u8 = 0; var c: u16 = 0; var d: i8 = 0;
fn square(x:u8)->u8 { return x*x; }
fn init(){ a = square(17); b = 250 + 10; c = u16(40000) + u16(30000); d = i8(249) / i8(3); }
fn update() {}
"#;
    for optimize in [false, true] {
        let (c, mut n) = run(source, optimize, 8);
        assert_eq!(read(&c, &mut n, "a") & 255, 33);
        assert_eq!(read(&c, &mut n, "b") & 255, 4);
        assert_eq!(read(&c, &mut n, "c"), 4464);
        assert_eq!(read(&c, &mut n, "d") & 255, 254);
    }
}
#[test]
fn loops_arrays_and_u16_indices() {
    let source = r#"
var table:[u8;300]=[0;300]; var total:u16=0; var flag:bool=false;
fn init(){for i in 0..300 {table[i]=u8(i);} for j in 0..300 {total += u16(table[j]);} flag = i16(65534) < i16(1);}
fn update() {}
"#;
    for optimize in [false, true] {
        let (c, mut n) = run(source, optimize, 30);
        assert_eq!(read(&c, &mut n, "total"), 33586);
        assert_eq!(read(&c, &mut n, "flag") & 255, 1);
    }
}
#[test]
fn asm_is_real_and_opaque() {
    let source = "var value:u8=0; fn init(){value=asm(a=128){asl a\nadc #$00\n};} fn update(){}";
    let (c, mut n) = run(source, true, 8);
    assert_eq!(read(&c, &mut n, "value") & 255, 1);
    assert!(c.generated.assembly.contains("asl a\nadc #$00"));
}
#[test]
fn asm_symbols_do_not_replace_opcodes_registers_or_literals() {
    let source = r#"
var a:u8=7; var x:u8=9; var lda:u8=3; var ff:u8=20;
var data:[u8;1]=[73];
var rotate:u8=0; var indexed:u8=0; var literal:u8=0; var named:u8=0;
fn init(){
    rotate=asm(a=128){asl a
        adc #$00};
    indexed=asm(x=0){lda data,x};
    literal=asm{lda #$ff};
    named=asm{ // lda x: JSR init
        lda a};
}
fn update(){}
"#;
    for optimize in [false, true] {
        let (c, mut n) = run(source, optimize, 8);
        for (name, expected) in [
            ("rotate", 1),
            ("indexed", 73),
            ("literal", 255),
            ("named", 7),
            ("a", 7),
        ] {
            assert_eq!(read(&c, &mut n, name) & 255, expected, "{name}");
        }
    }
    let invalid = "var a:u8=0; fn update(){a+=1;} @nmi fn v(){asm{inc a};}";
    assert!(compile(invalid, &Assets::new(), Options::default()).is_err());
}
#[test]
fn rejects_bounds_recursion_and_types() {
    for source in [
        "var x:u8=0; fn update(){x=300;}",
        "var a:[u8;4]=[0;4]; fn update(){a[4]=0;}",
        "fn f(){f();} fn update(){f();}",
        "var x:u8=0; fn update(){x=u16(3);}",
    ] {
        assert!(
            compile(source, &Assets::new(), Options::default()).is_err(),
            "{source}"
        );
    }
}
#[test]
fn conservative_budget_is_enforced() {
    let yes = "@budget(500) fn update(){poke(0x4000,1);}";
    assert!(
        compile(yes, &Assets::new(), Options::default())
            .unwrap()
            .verified_budgets["update"]
            < 500
    );
    let no = yes.replace("500", "1");
    assert!(compile(&no, &Assets::new(), Options::default()).is_err());
}
#[test]
fn mapper_banks_execute() {
    for mapper in ["nrom", "mmc1", "mmc3"] {
        let source =
            format!("cartridge {{mapper={mapper};}} var tick:u8=0; fn update(){{tick+=1;}}");
        let (c, mut n) = run(&source, true, 12);
        assert!((read(&c, &mut n, "tick") & 255) > 4, "{mapper}");
    }
    let source="cartridge{mapper=uxrom;chr=\"tiles.chr\";} @bank(1) const a:[u8;1]=[73]; var value:u8=0; fn init(){value=a[0];} fn update(){}";
    let mut assets = Assets::new();
    assets.insert("tiles.chr".into(), vec![0; 8192]);
    let c = compile(source, &assets, Options::default()).unwrap();
    let mut n = Emulator::new(c.rom.clone(), 44100.0).unwrap();
    for _ in 0..12 {
        n.frame(0).unwrap();
    }
    assert_eq!(read(&c, &mut n, "value") & 255, 73);
}
#[test]
fn interrupt_contexts_and_bounds_are_checked() {
    let invalid=[
  "var x:u8=0; fn update(){x+=1;} @nmi fn v(){x+=1;}",
  "@shared var x:u16=0; fn update(){} @nmi fn v(){x+=1;}",
  "fn h(){asm{nop};} fn update(){h();} @nmi fn v(){h();}",
  "fn h(){asm{nop};} fn update(){} @nmi fn v(){h();} @irq fn i(){h();}",
  "var a:[u8;4]=[0;4]; var i:u8=0; fn h(){i=8;} fn update(){if i<4{for k in 0..2{h();}a[i]=1;}}",
  "var a:[u8;4]=[0;4]; fn update(){var i:u8=0;while i<10{a[i]=1;i+=1;}}",
  "@shared var i:u8=0; var a:[u8;4]=[0;4]; fn update(){if i<4{a[i]=1;}}",
  "fn update(){} @nmi fn v(){var address:u16=0x4000;poke(address,1);}",
 ];
    for source in invalid {
        assert!(
            compile(source, &Assets::new(), Options::default()).is_err(),
            "{source}"
        );
    }
    let source="@shared var x:u8=0; fn update(){x+=1;} @nmi fn v(){let a:u8=x+1; x=a;} @irq fn i(){let b:u8=x+2; x=b;}";
    let c = compile(source, &Assets::new(), Options::default()).unwrap();
    assert_ne!(c.assembly.symbols["__v_v_a"], c.assembly.symbols["__v_i_b"]);
}
#[test]
fn optimized_arithmetic_matches_modular_reference() {
    let source = r#"var checksum:u16=0;var result:[u8;256]=[0;256];fn init(){for i in 0..256{let x:u8=u8(i);result[i]=((x*7)+13)^((x-9)&31);checksum+=u16(result[i]);}}fn update(){}"#;
    let expected = (0..=255u8)
        .map(|x| (x.wrapping_mul(7).wrapping_add(13)) ^ ((x.wrapping_sub(9)) & 31))
        .collect::<Vec<_>>();
    for optimize in [false, true] {
        let (c, mut n) = run(source, optimize, 90);
        let address = c.assembly.symbols["__v_result"];
        let actual = (0..256)
            .map(|i| n.read_ram(address + i))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(
            read(&c, &mut n, "checksum"),
            expected.iter().map(|x| u16::from(*x)).sum::<u16>()
        );
    }
}
#[test]
fn bank_read_side_effect_survives_zero_multiplication() {
    let source="cartridge{mapper=mmc3;} @bank(1) const a:[u8;1]=[73]; var x:u8=0;fn init(){x=a[0]*0;}fn update(){}";
    let c = compile(source, &Assets::new(), Options::default()).unwrap();
    assert!(c.generated.bank_reads > 0);
}

#[test]
fn compound_index_evaluates_once() {
    let source = "var count:u8=0;var table:[u8;4]=[0;4];fn index()->u8{count+=1;return count;}fn init(){table[raw index()]+=5;}fn update(){}";
    for optimize in [false, true] {
        let (c, mut n) = run(source, optimize, 10);
        assert_eq!(read(&c, &mut n, "count") & 255, 1);
        assert_eq!(n.read_ram(c.assembly.symbols["__v_table"] + 1), 5);
    }
}
#[test]
fn optimization_reduces_code_and_preserves_state() {
    let source="var total:u8=0;fn scale(x:u8)->u8{return x*4+0;}fn unused(){poke(0x4000,7);}fn init(){for i in 0..4{total+=scale(i);}if false{total=99;}}fn update(){}";
    let (a, mut an) = run(source, false, 10);
    let (b, mut bn) = run(source, true, 10);
    assert_eq!(read(&a, &mut an, "total") & 255, 24);
    assert_eq!(read(&b, &mut bn, "total") & 255, 24);
    assert!(b.assembly.bytes.len() < a.assembly.bytes.len());
    assert!(b.optimizations.inlined > 0);
    assert!(b.optimizations.unrolled > 0);
    assert!(b.optimizations.dead_functions > 0);
}

#[test]
fn assembly_calls_allocate_callee_frames() {
    let source="var result:u8=0;fn helper(){var scratch:u8=21;result=scratch*2;}fn init(){var keep:u8=73;asm{JSR helper};result+=keep;}fn update(){}";
    let (c, mut n) = run(source, true, 12);
    assert_eq!(read(&c, &mut n, "result") & 255, 115);
    assert_ne!(
        c.assembly.symbols["__v_init_keep"],
        c.assembly.symbols["__v_helper_scratch"]
    );
}
#[test]
fn interrupt_software_math_cannot_corrupt_main_scratch() {
    let source = "@shared var x:u8=0;fn update(){}@nmi fn tick(){x=x/3;}";
    assert!(compile(source, &Assets::new(), Options::default()).is_err());
}

#[test]
fn bounds_guard_cannot_survive_a_mutating_boolean_call() {
    let source="var i:u8=0;var a:[u8;4]=[0;4];fn mutate()->bool{i=8;return true;}fn update(){if i<4&&mutate(){a[i]=1;}}";
    assert!(compile(source, &Assets::new(), Options::default()).is_err());
}
#[test]
fn deep_previsited_call_graph_is_rejected() {
    let mut source = "fn f0(){}".to_owned();
    for i in 1..70 {
        source.push_str(&format!("fn f{i}(){{f{}();}}", i - 1));
    }
    source.push_str("fn update(){f69();}");
    assert!(compile(&source, &Assets::new(), Options::default()).is_err());
}
