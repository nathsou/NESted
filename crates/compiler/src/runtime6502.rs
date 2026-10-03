//! The small, explicit standard runtime. Main and NMI communicate through a
//! one-byte publication marker. NMI never touches runtime scratch before a
//! completed frame is published. DMC playback is disabled by this runtime.
use crate::{codegen::Mapper,frontend::Program};
pub fn runtime(mapper:Mapper,p:&Program,initializers:&str,reset:Option<&str>,nmi:Option<&str>,irq:Option<&str>,has_init:bool)->String{
 let mut s=String::from(r#"
__reset:
 sei
 cld
 ldx #255
 txs
 lda #0
 sta $2000
 sta $2001
 sta $4010
 lda #64
 sta $4017
 ldx #0
__clear_ram:
 lda #0
 sta $0000,x
 sta $0100,x
 sta $0200,x
 sta $0300,x
 sta $0400,x
 sta $0500,x
 sta $0600,x
 sta $0700,x
 inx
 bne __clear_ram
 lda #225
 sta $1c
 lda #172
 sta $1d
__warmup1:
 bit $2002
 bpl __warmup1
__warmup2:
 bit $2002
 bpl __warmup2
"#);
 let vertical=p.config.get("mirroring").is_none_or(|s|s=="vertical");
 match mapper{
 Mapper::Mmc1=>s.push_str(&format!("lda #128\nsta $8000\nlda #{}\nldx #5\n__mmc1_init:\nsta $8000\nlsr a\ndex\nbne __mmc1_init\nlda #0\nsta $04\njsr __rt_bank\n",if vertical{14}else{15})),
 Mapper::Mmc3=>{s.push_str("lda #0\nsta $e000\nlda #0\nsta $8000\nlda #0\nsta $8001\nlda #1\nsta $8000\nlda #2\nsta $8001\n");for i in 2..6{s.push_str(&format!("lda #{i}\nsta $8000\nlda #{}\nsta $8001\n",i+2));}s.push_str(&format!("lda #{}\nsta $a000\nlda #128\nsta $a001\n",u8::from(!vertical)));},
 Mapper::Uxrom=>s.push_str("lda #0\nsta $04\njsr __rt_bank\n"),_=>{}
 }
 if mapper==Mapper::Uxrom{s.push_str(r#"
 bit $2002
 lda #0
 sta $2006
 sta $2006
 sta $0a
 lda #128
 sta $0b
 ldx #32
 ldy #0
__load_chr_ram:
 lda ($0a),y
 sta $2007
 iny
 bne __load_chr_ram
 inc $0b
 dex
 bne __load_chr_ram
"#);}
 s.push_str(initializers);
 if p.config.contains_key("screen"){s.push_str("lda #<__initial_screen\nsta $04\nlda #>__initial_screen\nsta $05\njsr __rt_screen\n");}
 if p.config.contains_key("palette"){s.push_str("bit $2002\nlda #63\nsta $2006\nlda #0\nsta $2006\nldx #0\n__load_palette:\nlda __initial_palette,x\nsta $2007\ninx\ncpx #32\nbne __load_palette\n");}
 s.push_str("jsr __rt_hide_sprites\nlda #15\nsta $4015\nlda #8\nsta $4001\nsta $4005\n");
 if let Some(reset)=reset{s.push_str(&format!("jsr __fn_{reset}\n__halt:\njmp __halt\n"));}
 else{
   if has_init{s.push_str("jsr __fn_init\n");}
   s.push_str(r#"
 bit $2002
 lda #0
 sta $2005
 sta $2005
 lda #128
 sta $2000
 lda #30
 sta $2001
 lda #1
 sta $0c
__main:
 jsr __rt_poll
 jsr __fn_update
 lda #1
 sta $00
__await_publication:
 lda $00
 bne __await_publication
 jmp __main
"#);
 }
 s.push_str(r#"
__nmi:
 pha
 txa
 pha
 tya
 pha
 inc $037e
"#);
 if reset.is_none(){s.push_str(r#"
 lda $00
 beq __nmi_done
 bit $2002
 ldx #0
__flush_queue:
 cpx $0d
 beq __flush_done
 lda $0300,x
 sta $2006
 inx
 lda $0300,x
 sta $2006
 inx
 lda $0300,x
 sta $2007
 inx
 jmp __flush_queue
__flush_done:
 lda #0
 sta $0d
 sta $2003
 lda #2
 sta $4014
 lda #0
 sta $2005
 sta $2005
 lda #128
 sta $2000
"#);}
 if let Some(name)=nmi{s.push_str(&format!("jsr __fn_{name}\n"));}
 if reset.is_none(){s.push_str("lda #0\nsta $00\n");}
 s.push_str(r#"
__nmi_done:
 pla
 tay
 pla
 tax
 pla
 rti
__irq:
"#);
 if let Some(name)=irq{s.push_str(&format!("pha\ntxa\npha\ntya\npha\njsr __fn_{name}\npla\ntay\npla\ntax\npla\n"));}
 s.push_str(r#"
 rti

__rt_poll:
 lda #1
 sta $4016
 lda #0
 sta $4016
 sta $01
 ldx #8
__poll_bit:
 lda $4016
 lsr a
 ror $01
 dex
 bne __poll_bit
 lda $03
 eor #255
 and $01
 sta $02
 lda $01
 sta $03
 rts

__rt_tile:
 lda $04
 cmp #32
 bcs __tile_done
 lda $05
 cmp #30
 bcs __tile_done
 and #7
 asl a
 asl a
 asl a
 asl a
 asl a
 clc
 adc $04
 sta $0e
 lda $05
 lsr a
 lsr a
 lsr a
 clc
 adc #32
 sta $0f
 lda $0c
 beq __tile_direct
 ldx $0d
 cpx #120
 bcs __queue_full
 lda $0f
 sta $0300,x
 inx
 lda $0e
 sta $0300,x
 inx
 lda $06
 sta $0300,x
 inx
 stx $0d
__tile_done:
 rts
__queue_full:
 inc $037f
 rts
__tile_direct:
 bit $2002
 lda $0f
 sta $2006
 lda $0e
 sta $2006
 lda $06
 sta $2007
 rts

__rt_palette:
 lda $04
 and #31
 sta $0e
 lda $0c
 beq __palette_direct
 ldx $0d
 cpx #120
 bcs __queue_full
 lda #63
 sta $0300,x
 inx
 lda $0e
 sta $0300,x
 inx
 lda $05
 sta $0300,x
 inx
 stx $0d
 rts
__palette_direct:
 bit $2002
 lda #63
 sta $2006
 lda $0e
 sta $2006
 lda $05
 sta $2007
 rts

__rt_text:
 lda $06
 sta $10
 lda $07
 sta $11
 lda $08
 sta $12
 lda $04
 sta $13
 lda $05
 sta $14
__text_loop:
 lda $12
 beq __text_done
 ldy #0
 lda ($10),y
 sta $06
 lda $13
 sta $04
 lda $14
 sta $05
 jsr __rt_tile
 inc $13
 inc $10
 bne __text_no_carry
 inc $11
__text_no_carry:
 dec $12
 jmp __text_loop
__text_done:
 rts

__rt_sprite:
 lda $04
 cmp #64
 bcs __sprite_done
 asl a
 asl a
 tax
 lda $06
 sec
 sbc #1
 sta $0200,x
 lda $07
 sta $0201,x
 lda $08
 sta $0202,x
 lda $05
 sta $0203,x
__sprite_done:
 rts
__rt_hide_sprites:
 ldx #0
 lda #255
__hide_loop:
 sta $0200,x
 inx
 inx
 inx
 inx
 bne __hide_loop
 rts

__rt_render:
 lda $04
 bne __render_on
 lda #0
 sta $2001
 sta $0c
 rts
__render_on:
 bit $2002
 lda #0
 sta $2005
 sta $2005
 lda #128
 sta $2000
 lda #30
 sta $2001
 lda #1
 sta $0c
 rts

__rt_screen:
 lda #0
 sta $2001
 bit $2002
 lda #32
 sta $2006
 lda #0
 sta $2006
 lda $04
 sta $0a
 lda $05
 sta $0b
 lda #4
 sta $0e
 ldy #0
__screen_copy:
 lda ($0a),y
 sta $2007
 iny
 bne __screen_copy
 inc $0b
 dec $0e
 bne __screen_copy
 lda $0c
 beq __screen_done
 lda #1
 sta $04
 jsr __rt_render
__screen_done:
 rts

__rt_copy_ram:
 lda $12
 ora $13
 beq __copy_done
 ldy #0
 lda ($0a),y
 sta ($10),y
 inc $0a
 bne __copy_src_ok
 inc $0b
__copy_src_ok:
 inc $10
 bne __copy_dst_ok
 inc $11
__copy_dst_ok:
 lda $12
 bne __copy_low
 dec $13
__copy_low:
 dec $12
 jmp __rt_copy_ram
__copy_done:
 rts

__rt_tone:
 lda $04
 cmp #2
 beq __triangle
 bcs __tone_done
 asl a
 asl a
 tax
 lda $07
 and #15
 ora #176
 sta $4000,x
 lda $05
 sta $4002,x
 lda $06
 and #7
 ora #8
 sta $4003,x
__tone_done:
 rts
__triangle:
 lda $07
 beq __triangle_silent
 lda #255
 sta $4008
 lda $05
 sta $400a
 lda $06
 and #7
 ora #8
 sta $400b
 rts
__triangle_silent:
 lda #128
 sta $4008
 rts
__rt_noise:
 lda $05
 and #15
 ora #48
 sta $400c
 lda $04
 and #15
 sta $400e
 lda #8
 sta $400f
 rts
__rt_silence:
 lda $04
 cmp #2
 beq __triangle_silent
 cmp #3
 beq __noise_silent
 cmp #2
 bcs __silence_done
 asl a
 asl a
 tax
 lda #176
 sta $4000,x
__silence_done:
 rts
__noise_silent:
 lda #48
 sta $400c
 rts

__rt_rand:
 lsr $1d
 ror $1c
 bcc __rand_done
 lda $1d
 eor #180
 sta $1d
__rand_done:
 lda $1c
 rts

__rt_mul:
 lda $04
 sta $17
 lda $05
 sta $18
 lda $06
 sta $19
 lda $07
 sta $1a
 lda #0
 sta $0f
 sta $1f
 ldx #16
__mul_loop:
 lsr $1a
 ror $19
 bcc __mul_skip
 clc
 lda $0f
 adc $17
 sta $0f
 lda $1f
 adc $18
 sta $1f
__mul_skip:
 asl $17
 rol $18
 dex
 bne __mul_loop
 lda $0f
 ldx $1f
 rts

__rt_div:
 lda $04
 sta $17
 lda $05
 sta $18
 lda $06
 sta $19
 lda $07
 sta $1a
 lda #0
 sta $0f
 sta $1f
 ldx #16
__div_loop:
 asl $17
 rol $18
 rol $0f
 rol $1f
 lda $1f
 cmp $1a
 bcc __div_skip
 bne __div_sub
 lda $0f
 cmp $19
 bcc __div_skip
__div_sub:
 sec
 lda $0f
 sbc $19
 sta $0f
 lda $1f
 sbc $1a
 sta $1f
 inc $17
__div_skip:
 dex
 bne __div_loop
 lda $17
 ldx $18
 rts
"#);
 match mapper{
 Mapper::Nrom=>s.push_str("__rt_bank:\nrts\n"),
 Mapper::Uxrom=>s.push_str("__rt_bank:\nlda $04\nsta $1e\ntax\nsta __bank_bus_values,x\nrts\n__bank_bus_values:\n.byte 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15\n"),
 Mapper::Mmc1=>s.push_str("__rt_bank:\nlda $04\nsta $1e\nldx #5\n__bank_serial:\nsta $e000\nlsr a\ndex\nbne __bank_serial\nrts\n"),
 Mapper::Mmc3=>s.push_str("__rt_bank:\nlda #6\nsta $8000\nlda $04\nsta $1e\nsta $8001\nrts\n"),
 }
 s.push_str("__rt_screen_bank:\njsr __rt_bank\nlda $05\nsta $04\nlda $06\nclc\nadc #128\nsta $05\njsr __rt_screen\nrts\n__rt_bank_peek:\njsr __rt_bank\nlda $05\nsta $0a\nlda $06\nclc\nadc #128\nsta $0b\nldy #0\nlda ($0a),y\nrts\n");
 s
}
