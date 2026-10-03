//! NESted's dependency-free compiler and shared language services.
pub mod assembler;
pub mod codegen;
pub mod frontend;
pub mod json;
pub mod lsp;
pub mod optimizer;
mod runtime6502;
use std::collections::{BTreeMap,BTreeSet};
use frontend::*;
use codegen::*;
use json::Json;

pub type Assets=BTreeMap<String,Vec<u8>>;
#[derive(Clone,Copy,Debug)]pub struct Options{pub optimize:bool}
impl Default for Options{fn default()->Self{Self{optimize:true}}}
#[derive(Clone,Debug)]pub struct Pass{pub name:String,pub text:String}
#[derive(Clone,Debug)]pub struct Compilation{
 pub rom:Vec<u8>,pub assembly:assembler::Assembly,pub passes:Vec<Pass>,pub generated:Generated,pub diagnostics:Vec<Diagnostic>,pub optimizations:optimizer::Optimizations,pub machine_changes:usize,pub prg_bytes:usize,pub chr_bytes:usize,pub verified_budgets:BTreeMap<String,usize>,
}
fn compile_error(message:impl Into<String>)->Vec<Diagnostic>{vec![Diagnostic::error(Span::default(),message)]}
pub fn compile(source:&str,assets:&Assets,options:Options)->Result<Compilation,Vec<Diagnostic>>{
 let analysis=analyze(source);if analysis.diagnostics.iter().any(|d|!d.warning){return Err(analysis.diagnostics);}let mut program=analysis.program.unwrap();
 let mut passes=vec![Pass{name:"01 · tokens".into(),text:program.tokens.iter().map(|t|format!("{:>6}..{:<6} {} {:?}",t.span.start,t.span.end,if t.string{"string"}else{"token "},t.text)).collect::<Vec<_>>().join("\n")},Pass{name:"02 · typed IR".into(),text:optimizer::dump(&program)}];
 let mut constants=BTreeMap::new();
 for global in &mut program.globals{
    let t=if let Ty::Array(t,_)=&global.ty{t.as_ref()}else{&global.ty};
    let encode=|e:&Expr|->Result<Vec<u8>,Vec<Diagnostic>>{let n=eval_const(e,&constants).ok_or_else(||vec![Diagnostic::error(e.span,"Initializer is not constant")])?;let mut bytes=vec![n as u8];if t.size()==2{bytes.push((n>>8)as u8);}Ok(bytes)};
    global.data=match &global.initial{Initial::Scalar(e)=>encode(e)?,Initial::List(v)=>{let mut out=Vec::new();for e in v{out.extend(encode(e)?);}out},Initial::Repeat(e,n)=>encode(e)?.repeat(*n),Initial::Asset(path)=>assets.get(path).cloned().ok_or_else(||vec![Diagnostic::error(global.span,format!("Missing asset '{path}'"))])?};
    if global.data.len()!=global.ty.size(){return Err(vec![Diagnostic::error(global.span,format!("Asset/initializer has {} bytes; {} requires {}",global.data.len(),global.ty,global.ty.size()))]);}
    if global.constant{if let Initial::Scalar(e)=&global.initial{constants.insert(global.name.clone(),global.ty.wrap(eval_const(e,&constants).unwrap()));}}
 }
 let optimizations=if options.optimize{optimizer::optimize(&mut program)}else{optimizer::Optimizations::default()};
 passes.push(Pass{name:"03 · optimized IR".into(),text:optimizer::dump(&program)});
 let mut generated=generate(&program,assets).map_err(|d|vec![d])?;
 passes.push(Pass{name:"04 · machine lowering".into(),text:generated.assembly.clone()});
 let machine_changes=if options.optimize{let (s,n)=assembler::optimize_machine(&generated.assembly);generated.assembly=s;n}else{0};
 passes.push(Pass{name:"05 · machine optimization".into(),text:generated.assembly.clone()});
 let assembly=assembler::assemble(&generated.assembly,generated.mapper.base()).map_err(compile_error)?;
 let capacity=if generated.mapper==Mapper::Nrom{32768}else{16384};
 if assembly.bytes.len()>capacity-6{return Err(compile_error(format!("Fixed code/data uses {} bytes, exceeding {} bytes for {:?}; place large const arrays in @bank(N)",assembly.bytes.len(),capacity-6,generated.mapper)));}
 let chr=if generated.mapper==Mapper::Uxrom{Vec::new()}else if let Some(path)=program.config.get("chr"){let data=assets.get(path).ok_or_else(||compile_error(format!("Missing CHR asset '{path}'")))?;if data.len()!=8192{return Err(compile_error("CHR asset must contain exactly 8192 bytes"));}data.clone()}else{vec![0;8192]};
 let max_bank=generated.bank_data.keys().max().copied().unwrap_or(0)as usize;
 let mut prg=if generated.mapper==Mapper::Nrom{vec![0xff;32768]}else{
    let fixed=if generated.mapper==Mapper::Mmc3{2}else{1};let banks=(max_bank+1+fixed).max(if fixed==2{4}else{2}).next_power_of_two();
    if (generated.mapper==Mapper::Mmc3&&banks>64)||(generated.mapper!=Mapper::Mmc3&&banks>16){return Err(compile_error("Bank allocation exceeds the declared board's PRG capacity"));}
    vec![0xff;banks*generated.mapper.bank_size()]
 };
 for (bank,data)in &generated.bank_data{let offset=*bank as usize*generated.mapper.bank_size();prg[offset..offset+data.len()].copy_from_slice(data);}
 let code_offset=prg.len()-capacity;prg[code_offset..code_offset+assembly.bytes.len()].copy_from_slice(&assembly.bytes);
 for (i,name)in ["__nmi","__reset","__irq"].iter().enumerate(){let address=assembly.symbols[*name];let offset=prg.len()-6+i*2;prg[offset]=address as u8;prg[offset+1]=(address>>8)as u8;}
 let mut header=vec![0;16];header[..4].copy_from_slice(b"NES\x1a");header[4]=(prg.len()/16384)as u8;header[5]=(chr.len()/8192)as u8;let vertical=program.config.get("mirroring").is_none_or(|s|s=="vertical");header[6]=(generated.mapper.id()<<4)|u8::from(vertical);header[8]=u8::from(matches!(generated.mapper,Mapper::Mmc1|Mapper::Mmc3));
 let prg_bytes=prg.len();let chr_bytes=chr.len();let mut rom=header;rom.extend(prg);rom.extend(chr);
 let mut verified_budgets=BTreeMap::new();
 for f in &program.functions{if let Some(budget)=f.attrs.get("budget"){let limit=budget.parse::<usize>().map_err(|_|vec![Diagnostic::error(f.name_span,"@budget requires an integer instruction-cycle limit")])?;let max=instruction_bound(&format!("__fn_{}",f.name),&assembly,&mut BTreeSet::new()).map_err(|message|vec![Diagnostic::error(f.name_span,message)])?;if max>limit{return Err(vec![Diagnostic::error(f.name_span,format!("Verified unstalled instruction bound is {max} cycles, exceeding budget {limit}"))]);}verified_budgets.insert(f.name.clone(),max);}}
 let mut layout=format!("mapper {:?} ({}), fixed CPU base ${:04X}\nPRG {} bytes · CHR {} bytes · ROM {} bytes\nfixed code/data {} / {} bytes\ncompiler RAM {} bytes · zero page {} bytes · overlaid frame peak {} bytes\ncall depth {} · conservative hardware stack allowance {} bytes\n\n",generated.mapper,generated.mapper.id(),generated.mapper.base(),prg_bytes,chr_bytes,rom.len(),assembly.bytes.len(),capacity-6,generated.ram_bytes,generated.zero_page_bytes,generated.frame_bytes,generated.call_depth,generated.call_depth*2+16);
 for m in &generated.memory{layout.push_str(&format!("${:04X} {:>4} bytes · {:<28} {}\n",m.address,m.size,m.name,m.kind));}for (bank,data)in &generated.bank_data{layout.push_str(&format!("PRG bank {bank}: {} bytes at CPU $8000\n",data.len()));}
 passes.push(Pass{name:"06 · placement & link".into(),text:layout});passes.push(Pass{name:"07 · assembled ROM".into(),text:assembly.listing.clone()});
 Ok(Compilation{rom,assembly,passes,generated,diagnostics:analysis.diagnostics,optimizations,machine_changes,prg_bytes,chr_bytes,verified_budgets})
}
/// Conservative longest-path instruction bound for acyclic, directly called code.
/// DMA, interrupt latency and PPU phase are deliberately outside this contract.
fn instruction_bound(name:&str,a:&assembler::Assembly,active:&mut BTreeSet<u16>)->Result<usize,String>{
 let address=*a.symbols.get(name).ok_or_else(||format!("Unknown timing entry {name}"))?;
 fn walk(pc:u16,a:&assembler::Assembly,active:&mut BTreeSet<u16>,memo:&mut BTreeMap<u16,usize>)->Result<usize,String>{
    if let Some(n)=memo.get(&pc){return Ok(*n);}if !active.insert(pc){return Err("Cannot verify budget: cyclic control flow requires a statically bounded/unrolled loop".into());}
    let instruction=a.instructions.iter().find(|i|i.address==pc).ok_or("Cannot verify budget: execution enters data or an indirect target")?;
    let mut split=instruction.text.split_whitespace();let op=split.next().unwrap();let arg=split.next().unwrap_or("");let next=pc.wrapping_add(instruction.bytes.len()as u16);
    let rest=match op{"rts"|"rti"=>0,"jmp"=>{if arg.starts_with('('){return Err("Cannot verify indirect jump".into());}walk(assembler::value(arg,&a.symbols).map_err(|s|s.to_string())?as u16,a,active,memo)?},"jsr"=>{let callee=walk(assembler::value(arg,&a.symbols)?as u16,a,active,memo)?;callee+walk(next,a,active,memo)?},"bcc"|"bcs"|"beq"|"bne"|"bmi"|"bpl"|"bvc"|"bvs"=>{let target=assembler::value(arg,&a.symbols)?as u16;walk(target,a,active,memo)?.max(walk(next,a,active,memo)?)},"brk"=>return Err("Cannot verify BRK control flow".into()),_=>walk(next,a,active,memo)?};
    active.remove(&pc);let n=rest+instruction.cycles_max as usize;memo.insert(pc,n);Ok(n)
 }walk(address,a,active,&mut BTreeMap::new())
}
pub fn asset_paths(source:&str)->Vec<String>{let Ok(p)=parse(source)else{return Vec::new();};let mut paths=BTreeSet::new();for key in ["chr","screen","palette"]{if let Some(path)=p.config.get(key){paths.insert(path.clone());}}for g in &p.globals{if let Initial::Asset(path)=&g.initial{paths.insert(path.clone());}}
 fn expr(e:&Expr,out:&mut BTreeSet<String>){match &e.kind{ExprKind::Call(n,v)=>{if n=="screen"{if let Some(Expr{kind:ExprKind::String(s),..})=v.first(){out.insert(s.clone());}}for e in v{expr(e,out);}},ExprKind::Binary(_,a,b)|ExprKind::Index(a,b,_)=>{expr(a,out);expr(b,out);},ExprKind::Unary(_,e)=>expr(e,out),_=>{}}}
 fn block(body:&[Stmt],out:&mut BTreeSet<String>){for s in body{match s{Stmt::Local{value,..}|Stmt::Expr(value)=>expr(value,out),Stmt::Assign{value,..}=>expr(value,out),Stmt::If{condition,yes,no}=>{expr(condition,out);block(yes,out);block(no,out);},Stmt::While{condition,body}=>{expr(condition,out);block(body,out);},Stmt::For{start,end,body,..}=>{expr(start,out);expr(end,out);block(body,out);},Stmt::Loop(b)=>block(b,out),Stmt::Return(Some(e),_)=>expr(e,out),_=>{}}}}
 for f in &p.functions{block(&f.body,&mut paths);}paths.into_iter().collect()
}
impl Compilation{
 pub fn json(&self)->Json{let o=&self.optimizations;Json::object([
    ("ok",true.into()),("romBytes",self.rom.len().into()),("prgBytes",self.prg_bytes.into()),("chrBytes",self.chr_bytes.into()),("codeBytes",self.assembly.bytes.len().into()),("mapper",self.generated.mapper.id().into()),("ramBytes",self.generated.ram_bytes.into()),("zeroPageBytes",self.generated.zero_page_bytes.into()),("frameBytes",self.generated.frame_bytes.into()),("stackAllowance",(self.generated.call_depth*2+16).into()),
    ("passes",Json::Array(self.passes.iter().map(|p|Json::object([("name",p.name.clone().into()),("text",p.text.clone().into())])).collect())),
    ("symbols",Json::Object(self.assembly.symbols.iter().filter(|(n,_)|n.starts_with("__v_")||n.starts_with("__fn_")).map(|(n,v)|(n.clone(),(*v).into())).collect())),
    ("memory",Json::Array(self.generated.memory.iter().map(|m|Json::object([("name",m.name.clone().into()),("address",m.address.into()),("size",m.size.into()),("kind",m.kind.clone().into())])).collect())),
    ("optimizations",Json::object([("constantFolds",o.folded.into()),("algebraic",o.simplified.into()),("inlined",o.inlined.into()),("branches",o.branches.into()),("unreachable",o.unreachable.into()),("unrolled",o.unrolled.into()),("deadFunctions",o.dead_functions.into()),("machine",self.machine_changes.into()),("relaxedBranches",self.assembly.relaxed.into()),("bankReads",self.generated.bank_reads.into())])),
    ("budgets",Json::Object(self.verified_budgets.iter().map(|(n,v)|(n.clone(),(*v).into())).collect())),
 ])}
}
