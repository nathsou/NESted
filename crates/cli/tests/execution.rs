use nested_compiler::{compile,Assets,Options};
use nested_runtime::Emulator;
fn run(source:&str,optimize:bool,frames:usize)->(nested_compiler::Compilation,Emulator){let c=compile(source,&Assets::new(),Options{optimize}).unwrap_or_else(|e|panic!("{e:?}"));let mut n=Emulator::new(c.rom.clone(),44100.0).unwrap();for _ in 0..frames{n.frame(0).unwrap();}(c,n)}
fn read(c:&nested_compiler::Compilation,n:&mut Emulator,name:&str)->u16{let address=c.assembly.symbols[&format!("__v_{name}")];u16::from(n.read_ram(address))|u16::from(n.read_ram(address+1))<<8}
#[test]fn arithmetic_and_calls_execute(){let source=r#"
var a: u8 = 0; var b: u8 = 0; var c: u16 = 0; var d: i8 = 0;
fn square(x:u8)->u8 { return x*x; }
fn init(){ a = square(17); b = 250 + 10; c = u16(40000) + u16(30000); d = i8(249) / i8(3); }
fn update() {}
"#;for optimize in [false,true]{let(c,mut n)=run(source,optimize,8);assert_eq!(read(&c,&mut n,"a")&255,33);assert_eq!(read(&c,&mut n,"b")&255,4);assert_eq!(read(&c,&mut n,"c"),4464);assert_eq!(read(&c,&mut n,"d")&255,254);}}
#[test]fn loops_arrays_and_u16_indices(){let source=r#"
var table:[u8;300]=[0;300]; var total:u16=0; var flag:bool=false;
fn init(){for i in 0..300 {table[i]=u8(i);} for j in 0..300 {total += u16(table[j]);} flag = i16(65534) < i16(1);}
fn update() {}
"#;for optimize in [false,true]{let(c,mut n)=run(source,optimize,30);assert_eq!(read(&c,&mut n,"total"),33586);assert_eq!(read(&c,&mut n,"flag")&255,1);}}
#[test]fn asm_is_real_and_opaque(){let source="var value:u8=0; fn init(){value=asm(a=128){asl a\nadc #$00\n};} fn update(){}";let(c,mut n)=run(source,true,8);assert_eq!(read(&c,&mut n,"value")&255,1);assert!(c.generated.assembly.contains("asl a\nadc #$00"));}
#[test]fn rejects_bounds_recursion_and_types(){for source in ["var x:u8=0; fn update(){x=300;}","var a:[u8;4]=[0;4]; fn update(){a[4]=0;}","fn f(){f();} fn update(){f();}","var x:u8=0; fn update(){x=u16(3);}"]{assert!(compile(source,&Assets::new(),Options::default()).is_err(),"{source}");}}
#[test]fn conservative_budget_is_enforced(){let yes="@budget(500) fn update(){poke(0x4000,1);}";assert!(compile(yes,&Assets::new(),Options::default()).unwrap().verified_budgets["update"]<500);let no=yes.replace("500","1");assert!(compile(&no,&Assets::new(),Options::default()).is_err());}
#[test]fn mapper_banks_execute(){for mapper in ["nrom","mmc1","mmc3"]{let source=format!("cartridge {{mapper={mapper};}} var tick:u8=0; fn update(){{tick+=1;}}");let(c,mut n)=run(&source,true,12);assert!((read(&c,&mut n,"tick")&255)>4,"{mapper}");}
 let source="cartridge{mapper=uxrom;chr=\"tiles.chr\";} @bank(1) const a:[u8;1]=[73]; var value:u8=0; fn init(){value=a[0];} fn update(){}";let mut assets=Assets::new();assets.insert("tiles.chr".into(),vec![0;8192]);let c=compile(source,&assets,Options::default()).unwrap();let mut n=Emulator::new(c.rom.clone(),44100.0).unwrap();for _ in 0..12{n.frame(0).unwrap();}assert_eq!(read(&c,&mut n,"value")&255,73);
}
