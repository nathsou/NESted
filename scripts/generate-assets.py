#!/usr/bin/env python3
"""Original, deterministic NES pixel art and compositions. Standard library only.
Every sprite is authored on a 16x16 grid; output is real two-plane NES CHR.
"""
from pathlib import Path
import json, math, random, struct, zlib
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'games' / 'assets'
OUT.mkdir(parents=True, exist_ok=True)
FONT = {
'A': ['01110','10001','10001','11111','10001','10001','10001'],
'B': ['11110','10001','10001','11110','10001','10001','11110'],
'C': ['01111','10000','10000','10000','10000','10000','01111'],
'D': ['11110','10001','10001','10001','10001','10001','11110'],
'E': ['11111','10000','10000','11110','10000','10000','11111'],
'F': ['11111','10000','10000','11110','10000','10000','10000'],
'G': ['01111','10000','10000','10111','10001','10001','01111'],
'H': ['10001','10001','10001','11111','10001','10001','10001'],
'I': ['11111','00100','00100','00100','00100','00100','11111'],
'J': ['00111','00010','00010','00010','10010','10010','01100'],
'K': ['10001','10010','10100','11000','10100','10010','10001'],
'L': ['10000','10000','10000','10000','10000','10000','11111'],
'M': ['10001','11011','10101','10101','10001','10001','10001'],
'N': ['10001','11001','10101','10011','10001','10001','10001'],
'O': ['01110','10001','10001','10001','10001','10001','01110'],
'P': ['11110','10001','10001','11110','10000','10000','10000'],
'Q': ['01110','10001','10001','10001','10101','10010','01101'],
'R': ['11110','10001','10001','11110','10100','10010','10001'],
'S': ['01111','10000','10000','01110','00001','00001','11110'],
'T': ['11111','00100','00100','00100','00100','00100','00100'],
'U': ['10001','10001','10001','10001','10001','10001','01110'],
'V': ['10001','10001','10001','10001','10001','01010','00100'],
'W': ['10001','10001','10001','10101','10101','10101','01010'],
'X': ['10001','10001','01010','00100','01010','10001','10001'],
'Y': ['10001','10001','01010','00100','00100','00100','00100'],
'Z': ['11111','00001','00010','00100','01000','10000','11111'],
'0': ['01110','10001','10011','10101','11001','10001','01110'],
'1': ['00100','01100','00100','00100','00100','00100','01110'],
'2': ['01110','10001','00001','00010','00100','01000','11111'],
'3': ['11110','00001','00001','01110','00001','00001','11110'],
'4': ['00010','00110','01010','10010','11111','00010','00010'],
'5': ['11111','10000','10000','11110','00001','00001','11110'],
'6': ['01110','10000','10000','11110','10001','10001','01110'],
'7': ['11111','00001','00010','00100','01000','01000','01000'],
'8': ['01110','10001','10001','01110','10001','10001','01110'],
'9': ['01110','10001','10001','01111','00001','00001','01110'],
':': ['00000','00100','00100','00000','00100','00100','00000'],
'-': ['00000','00000','00000','11111','00000','00000','00000'],
'/': ['00001','00001','00010','00100','01000','10000','10000'],
'!': ['00100','00100','00100','00100','00100','00000','00100'],
'?': ['01110','10001','00001','00010','00100','00000','00100'],
'+': ['00000','00100','00100','11111','00100','00100','00000'],
'.': ['00000','00000','00000','00000','00000','00110','00110'],
'%': ['11001','11010','00100','01000','10110','00110','00000'],
'&': ['01100','10010','10100','01000','10101','10010','01101'],
'>': ['10000','01000','00100','00010','00100','01000','10000'],
'<': ['00001','00010','00100','01000','00100','00010','00001'],
'=': ['00000','00000','11111','00000','11111','00000','00000'],
' ': ['00000']*7,
}
PALETTES = {
'bloom': [0x30,0x0f,0x10,0x00]*4 + [0x30,0x0f,0x16,0x30]*4,
'starstring': [0x0f,0x30,0x12,0x2c, 0x0f,0x02,0x12,0x2c, 0x0f,0x04,0x14,0x34, 0x0f,0x06,0x16,0x28,
               0x0f,0x02,0x2c,0x30, 0x0f,0x04,0x34,0x30, 0x0f,0x06,0x28,0x30, 0x0f,0x05,0x26,0x30],
'skythread': [0x0f,0x30,0x12,0x2c, 0x0f,0x01,0x11,0x31, 0x0f,0x01,0x11,0x21, 0x0f,0x06,0x16,0x30,
             0x0f,0x0f,0x26,0x37, 0x0f,0x06,0x16,0x28, 0x0f,0x01,0x21,0x30, 0x0f,0x04,0x34,0x30],
'emberkeep': [0x0f,0x30,0x00,0x10, 0x0f,0x01,0x00,0x10, 0x0f,0x09,0x19,0x29, 0x0f,0x05,0x16,0x26,
             0x0f,0x0c,0x27,0x37, 0x0f,0x09,0x19,0x37, 0x0f,0x05,0x26,0x30, 0x0f,0x01,0x21,0x30],
}
def blank(n=8): return [[0]*n for _ in range(n)]
def pattern(rows):
    table={'0':0,'.':0,' ':0,'1':1,'2':2,'3':3,'#':1,'+':2,'@':3}
    return [[table[c] for c in row] for row in rows]
def split_sprite(tiles, index, img):
    for dy in range(2):
        for dx in range(2):
            tiles[index+dy*2+dx]=[row[dx*8:dx*8+8] for row in img[dy*8:dy*8+8]]
def encode_tile(tile):
    result=[]
    for bit in (0,1):
        for row in tile:
            result.append(sum(((pixel>>bit)&1)<<(7-x) for x,pixel in enumerate(row)))
    return bytes(result)
def rect(img,x,y,w,h,color):
    for yy in range(max(0,y),min(len(img),y+h)):
        for xx in range(max(0,x),min(len(img[0]),x+w)):img[yy][xx]=color
def common_tiles(game):
    tiles=[blank() for _ in range(256)]
    for c,rows in FONT.items():
        tile=blank()
        for y,row in enumerate(rows):
            for x,v in enumerate(row):
                if v=='1':tile[y][x+1]=1
        tiles[ord(c)]=tile
    for i in range(1,9):
        t=blank()
        for y in range(8):
            for x in range(8):
                if (i in (1,3,5) and y==0) or (i in (2,4,5) and y==7) or (i in (1,2,6) and x==0) or (i in (3,4,6) and x==7):t[y][x]=3
        tiles[i]=t
    tiles[16]=pattern(['........','........','...1....','..131...','...1....','........','........','........'])
    tiles[17]=pattern(['.1......','111....1','212...11','1211..21','.1211.12','..121112','...12121','....1111'])
    tiles[18]=pattern(['22222222','11111111','11111111','11111111','21112111','11111111','11111111','11111111'])
    tiles[19]=pattern(['........','........','...3....','..323...','.32223..','3222223.','22222223','11111111'])
    if game=='skythread':
        tiles[17]=[[1 for x in range(8)] for y in range(8)]
    tiles[20]=pattern(['....22..','....232.','....22..','....2...','..222...','.2332...','.222....','........'])
    # Puzzle cells: crisp rounded corners, subtle highlights and crossed marks.
    for index,kind in [(128,0),(132,1),(136,2)]:
        img=blank(16)
        for y in range(1,15):
            for x in range(1,15):
                if (x in (1,14) and y in (1,14)):continue
                img[y][x]=2 if kind==1 else 1
        if kind==1:
            rect(img,3,3,9,1,1);rect(img,3,4,1,6,1);rect(img,12,7,1,6,3)
        if kind==2:
            for k in range(4,12):img[k][k]=3;img[k][15-k]=3
        split_sprite(tiles,index,img)
    cursor=blank(16)
    for y in range(16):
        for x in range(16):
            if ((x in (0,15) and (y<5 or y>10)) or (y in (0,15) and (x<5 or x>10))):cursor[y][x]=2
            elif ((x in (1,14) and (y<4 or y>11)) or (y in (1,14) and (x<4 or x>11))):cursor[y][x]=3
    split_sprite(tiles,140,cursor)
    flower=pattern(['................','......1111......','.....122221.....','..111123322111..','.12222133122221.','.12333233233321.','.12333233233321.','..122233332221..','...1122222211...','.....111111.....','.......22.......','....22.22.22....','...2332222332...','....22222222....','.......22.......','......1111......'])
    split_sprite(tiles,144,flower)
    hero=pattern(['................','.....11111......','....1222221.....','...122222221....','...122222221....','...123333331....','...123131331....','....1333331.....','.....13331......','...112222211....','..12222222221...','..12232223221...','...123222321....','....1222221.....','....1311131.....','...111...111....'])
    if game=='emberkeep':
        hero=pattern(['.....111111.....','....12222221....','...1222222221...','...1221111221...','...1213333121...','...1213133121...','....11333311....','.....13331......','...112222211....','..12222222221...','..12232223221...','...123222321....','....1222221.....','....1311131.....','...111...111....','................'])
    split_sprite(tiles,148,hero)
    run1=[r[:] for r in hero];run2=[r[:] for r in hero]
    rect(run1,3,14,4,2,0);rect(run1,8,14,4,2,0);rect(run1,2,14,4,1,1);rect(run1,9,15,4,1,1)
    rect(run2,3,14,4,2,0);rect(run2,8,14,4,2,0);rect(run2,3,15,4,1,1);rect(run2,10,14,4,1,1)
    split_sprite(tiles,152,run1);split_sprite(tiles,156,run2)
    gem=pattern(['................','.......33.......','......3223......','.....322223.....','....32233223....','...3223333223...','...3233333323...','....22333322....','.....223322.....','......2222......','.......22.......','................','................','................','................','................'])
    split_sprite(tiles,160,gem)
    slime=pattern(['................','................','................','................','.....111111.....','...1122222211...','..122222222221..','.12233322333221.','.12331322313321.','.12333322333321.','.12222222222221.','.12222222222221.','..122222222221..','...1111111111...','................','................'])
    split_sprite(tiles,164,slime)
    bat=pattern(['................','.11..........11.','.1211......1121.','.122211..112221.','..122221122221..','..122222222221..','...1223333221...','....12311321....','....12222221....','.....122221.....','......1111......','.......11.......','................','................','................','................'])
    split_sprite(tiles,168,bat)
    chest=pattern(['................','................','..111111111111..','.12222222222221.','.12333333333321.','.12322222222321.','.12322222222321.','.11111133111111.','.12222233122221.','.12222222122221.','.12222222222221.','.12322222222321.','.12333333333321.','..111111111111..','................','................'])
    split_sprite(tiles,172,chest)
    gate=blank(16)
    for y in range(1,16):
        for x in range(2,14):
            if y<4 and (x<5-y or x>10+y):continue
            gate[y][x]=1 if x in (2,13) or y==1 else 2
    rect(gate,5,5,6,11,0);rect(gate,7,6,2,8,3)
    split_sprite(tiles,176,gate)
    spikes=blank(16)
    for k in (0,8):
        for y in range(4,16):
            for x in range(8):
                if abs(x-3.5)<=((y-4)/3):spikes[y][k+x]=3 if x<4 else 2
    split_sprite(tiles,180,spikes)
    ground=blank(16)
    for y in range(16):
        for x in range(16):
            ground[y][x]=3 if y==0 else 2 if y<3 else 1
            if y in (7,15) or ((x+(0 if y<8 else 8))%16==0 and y>3):ground[y][x]=0
            if (x,y) in [(3,5),(4,5),(11,12),(12,12)]:ground[y][x]=2
    split_sprite(tiles,184,ground)
    decor=blank(16)
    for y in range(16):
        for x in range(16):
            if y>8 and abs(x-8)<(y-7)/2:decor[y][x]=1 if (x+y)%3 else 2
    split_sprite(tiles,188,decor)
    for idx,hold in [(192,False),(196,True),(200,False)]:
        note=blank(16)
        for y in range(2,10):
            for x in range(1,15):
                if (x in (1,14) and y in (2,9)):continue
                note[y][x]=1 if x in (1,14) or y in (2,9) else 3 if y==3 else 2
        if hold:rect(note,6,10,4,6,2)
        if idx==200:
            for y in range(16):
                for x in range(16):
                    if x in (0,15) or y in (0,15):note[y][x]=3
        split_sprite(tiles,idx,note)
    dash=[r[:] for r in hero]
    for y in range(16):
        for x in range(16):
            if dash[y][x]==2:dash[y][x]=3
    split_sprite(tiles,208,dash)
    lantern=pattern(['................','.....111111.....','....12222221....','....12111121....','.....111111.....','.....123321.....','.....123321.....','....12333321....','....12333321....','....12333321....','.....123321.....','.....111111.....','................','................','................','................'])
    split_sprite(tiles,212,lantern)
    stairs=blank(16)
    for y in range(16):
        for x in range(16):stairs[y][x]=1
    for k in range(4):rect(stairs,2+k*2,3+k*3,12-k*2,2,3)
    split_sprite(tiles,216,stairs)
    floor=blank(16)
    for y in range(16):
        for x in range(16):floor[y][x]=1 if (x,y) not in [(2,3),(10,11),(7,8),(8,8)] else 2
    split_sprite(tiles,220,floor)
    moss=[r[:] for r in ground]
    for y in range(4):
        for x in range(16):moss[y][x]=2 if (x*7+y*3)%11<7 else 3
    split_sprite(tiles,224,moss)
    heart=pattern(['................','................','...111....111...','..12221..12221..','.12332211223321.','.12322222222321.','.12222222222221.','..122222222221..','...1222222221...','....12222221....','.....122221.....','......1221......','.......11.......','................','................','................'])
    split_sprite(tiles,232,heart)
    butterfly=pattern(['................','................','..111......111..','.12221....12221.','.123221..122321.','..122221122221..','...1222112221...','....11211211....','...1222112221...','..123221122321..','...1222112221...','....111..111....','................','................','................','................'])
    split_sprite(tiles,240,butterfly)
    tail=blank(16)
    rect(tail,5,0,6,16,2);rect(tail,6,0,1,16,3);rect(tail,9,0,1,16,1)
    split_sprite(tiles,244,tail)
    crown=pattern(['................','................','..3....3....3...','..23..323..32...','..22332223322...','..22222222222...','...222222222....','...333333333....','...222222222....','................','................','................','................','................','................','................'])
    split_sprite(tiles,248,crown)
    generated=json.loads((ROOT/'games/art/sprites.json').read_text())
    for name,index in [('mushroom',96),('flower',144),('hero',148),('run1',152),('run2',156),('gem',160),('slime',164),('bat',168),('chest',172),('gate',176),('terrain',184),('note',192),('lantern',212),('butterfly',240),('spirit',248)]:
        split_sprite(tiles,index,pattern(generated[name]))
    if game=='emberkeep':split_sprite(tiles,148,pattern(generated['explorer']))
    dash=pattern(generated['hero'])
    for row in dash:
        for x,value in enumerate(row):
            if value==2:row[x]=3
    split_sprite(tiles,208,dash)
    if game=='bloom':
        # Four border variants per state: emphasize the fifth row/column.
        for kind in range(3):
            for border in range(4):
                cell=blank(16)
                for y in range(16):
                    for x in range(16):
                        if x < (2 if border&1 else 1) or y < (2 if border&2 else 1):cell[y][x]=2
                        elif kind==1:cell[y][x]=1
                if kind==2:
                    for k in range(5,12):cell[k][k]=3;cell[k][16-k]=3
                split_sprite(tiles,128+kind*16+border*4,cell)
        for number in range(10):
            tiles[96+number]=[[2 if v else 0 for v in row] for row in tiles[48+number]]
        tiles[120]=[[2 if x==0 else 0 for x in range(8)]for y in range(8)]
        tiles[121]=[[2 if y==0 else 0 for x in range(8)]for y in range(8)]
        tiles[122]=[[2 if x==0 or y==0 else 0 for x in range(8)]for y in range(8)]
        split_sprite(tiles,196,cursor)
    if game=='starstring':
        tiles[192]=pattern(['...22...','..2332..','.233332.','23333332','.233332.','..2332..','...22...','........'])
        tiles[196]=pattern(['..2222..','.233332.','.232232.','.232232.','.233332.','..2222..','...22...','...22...'])
        tiles[200]=pattern(['...33...','..3333..','.332233.','33222233','.332233.','..3333..','...33...','........'])
        tiles[244]=pattern(['...22...','...32...','...32...','...32...','...32...','...32...','...32...','...22...'])
        tiles[249]=pattern(['........','........','........','22222222','22222222','........','........','........'])
        for i,rows in enumerate([
            ['...1....','..11....','.111111.','1111111.','.111111.','..11....','...1....','........'],
            ['...1....','...1....','...1....','.1.1.1..','..111...','...1....','........','........'],
            ['...1....','..111...','.1.1.1..','...1....','...1....','...1....','........','........'],
            ['...1....','...11...','.111111.','.1111111','.111111.','...11...','...1....','........']]):tiles[128+i]=pattern(rows)
    if game=='skythread':
        stone=blank(16)
        rect(stone,0,0,16,16,1);rect(stone,0,0,16,2,3);rect(stone,0,2,1,14,2)
        rect(stone,8,2,1,6,2);rect(stone,2,9,11,1,2);rect(stone,5,10,1,6,2)
        split_sprite(tiles,184,stone)
    if game=='emberkeep':
        floor=blank(16);rect(floor,0,0,16,16,1)
        for x,y in [(3,4),(11,10)]:floor[y][x]=2
        split_sprite(tiles,220,floor)
        wall=blank(16);rect(wall,0,0,16,16,2)
        rect(wall,0,7,16,1,1);rect(wall,7,0,1,7,1);rect(wall,3,8,1,8,1)
        rect(wall,0,0,16,1,3);rect(wall,0,8,16,1,3)
        split_sprite(tiles,224,wall)
    return tiles
class Screen:
    def __init__(self):self.data=[0]*960;self.pals=[0]*960
    def tile(self,x,y,t,pal=0):
        if 0<=x<32 and 0<=y<30:
            self.data[y*32+x]=t
            # One NES attribute quadrant covers 2x2 tiles, never one tile.
            for yy in range(y&~1,min(30,(y&~1)+2)):
                for xx in range(x&~1,(x&~1)+2):self.pals[yy*32+xx]=pal
    def text(self,x,y,text,pal=0):
        for i,c in enumerate(text.upper()):self.tile(x+i,y,ord(c),pal)
    def meta(self,x,y,index,pal=0):
        for dy in range(2):
            for dx in range(2):self.tile(x+dx,y+dy,index+dy*2+dx,pal)
    def box(self,x,y,w,h,pal=0):
        for xx in range(x+1,x+w-1):self.tile(xx,y,5,pal);self.tile(xx,y+h-1,5,pal)
        for yy in range(y+1,y+h-1):self.tile(x,yy,6,pal);self.tile(x+w-1,yy,6,pal)
        for dx,dy,t in [(0,0,1),(w-1,0,3),(0,h-1,2),(w-1,h-1,4)]:self.tile(x+dx,y+dy,t,pal)
    def encode(self):
        attrs=[]
        for ay in range(8):
            for ax in range(8):
                val=0
                for qy in range(2):
                    for qx in range(2):
                        x=ax*4+qx*2;y=ay*4+qy*2
                        if y<30:val|=self.pals[y*32+x]<<((qy*2+qx)*2)
                attrs.append(val)
        return bytes(self.data+attrs)

def nonogram():
    s=Screen()
    s.text(12,1,'NONOGRAM')
    s.text(8,2,'PUZZLE 01 / 16')
    for y in range(8,24):s.tile(26,y,120)
    for x in range(10,26):s.tile(x,24,121)
    s.tile(26,24,122)
    s.text(7,25,'PUZZLE IN PROGRESS')
    s.text(5,27,'A FILL  B MARK  SELECT UNDO')
    s.text(8,28,'START: NEXT PUZZLE')
    return s

def rhythm():
    s=Screen()
    s.text(11,1,'STARSTRING')
    s.text(5,4,'MOONLIGHT CIRCUIT / 112 BPM')
    s.text(3,6,'SCORE');s.text(22,6,'COMBO')
    for lane,x in enumerate((5,11,17,23)):
        for y in range(10,25):s.tile(x,y,6,1);s.tile(x+2,y,6,1)
        s.tile(x+1,25,128+lane)
    for x in range(3,29):s.tile(x,24,5,2)
    s.text(4,28,'ARROWS PLAY / START PAUSE')
    return s

ROOMS = [
['................','................','................','.............E..','.............##.','........G.......','...........###..','................','.........###....','................','......###.......','................','...###..........','..P.............','################'],
['................','................','..............E.','............###.','................','.........##.....','................','......##...G....','................','...##...........','................','................','..P....^^.......','#####..^^..#####','################'],
['................','................','.............E..','............###.','.........#......','.........#......','......G..#......','.....##..#......','.........#......','...##....#......','.........#......','.........#......','..P......#......','#####....#......','################'],
['................','................','.............E..','...........####.','.........G......','........##......','......^^........','.....####.......','................','..##............','.........^^.....','.......#####....','..P.............','#####...........','################'],
['................','................','.............E..','...........###..','................','........##......','....G...........','...##...........','................','.....##.........','.........#......','.........#......','..P......#......','#####....#..^^..','################'],
['................','................','..............E.','............####','.........G......','........##......','.....^^.........','....####........','................','..##............','.......##.......','............^^..','..P.......####..','#####...........','################'],
]
def mountain(room,index):
    s=Screen();rng=random.Random(180+index)
    for y in range(4,27):
        for x in range(32):
            if rng.randrange(32)==0:s.tile(x,y,16,2)
            if y>17+abs(x-12)//3 and s.data[y*32+x]==0:s.tile(x,y,17,2)
    s.text(2,1,'SKYTHREAD');s.text(18,1,f'ASCENT {index+1}/6');s.text(2,2,'A JUMP  B DASH');s.text(20,2,'DEATHS 000')
    for y,row in enumerate(room):
        for x,c in enumerate(row):
            if c=='#':s.meta(x*2,y*2,184,1)
            elif c=='^':s.meta(x*2,y*2,180,3)
            elif c=='E':s.meta(x*2,y*2,176,2)
            elif c=='G':pass  # Crystals are animated OAM sprites; collision data remains.
    return s

def dungeon():
    s=Screen()
    s.text(2,1,'EMBERKEEP');s.text(18,1,'LAST LANTERN')
    s.text(2,2,'FL 1 P2 HP 08 K - G 000')
    s.text(3,28,'ARROWS MOVE  A WAIT  B HEAL')
    for y in range(4,28):
        for x in range(32):s.tile(x,y,0,1)
    return s

PUZZLES=[
['00111100','01111110','11111111','11011011','11111111','00111100','00100100','01100110'],
['01100110','11111111','11111111','11111111','01111110','00111100','00011000','00000000'],
['01000010','11100111','11111111','11011011','11111111','01111110','00111100','00011000'],
['00011000','00111100','00111100','01111110','01111110','11111111','01011010','11000011'],
['00011000','00011000','01111110','11111111','01111110','00011000','00111100','01100110'],
['00111100','01000010','10111101','10100101','10111101','01011010','00100100','01100110'],
['11000011','11100111','01111110','00111100','01111110','11100111','11000011','00000000'],
['00011000','00111100','01111110','11111111','01111110','00111100','00011000','00011000'],
]
PUZZLES += [
['00000000','00111000','01111101','11111111','01111101','00111000','00000000','00000000'],
['00011110','00010010','00010010','00010000','00010000','01110000','11110000','01100000'],
['00000000','11111100','10000110','10000101','10000110','01111000','00000000','11111110'],
['00111000','01100100','01000100','00111000','00010000','00011000','00010000','00011000'],
['00011000','00111100','01111110','11111111','01000010','01011010','01011010','01111110'],
['00011000','00011000','00111100','00011000','01011010','10011001','11011011','01111110'],
['00000001','00000011','00000111','00001111','00011111','00111111','01111111','11111111'],
['11110000','11110000','00111100','00111100','00001111','00001111','11000011','11000111'],
]

PUZZLES = [PUZZLES[i] for i in [14,1,7,0,2,3,4,5,6,8,9,10,11,12,13,15]]

def clues(line):
    groups=[];run=0
    for v in line+'0':
        if v=='1':run+=1
        elif run:groups.append(run);run=0
    return groups or [0]
def nst_array(name,values,ty='u8'):
    return f'const {name}: [{ty}; {len(values)}] = [\n'+''.join('    '+', '.join(str(v) for v in values[i:i+16])+',\n' for i in range(0,len(values),16))+'];\n'
def period(note,triangle=False):
    if note<0:return 0
    return max(8,min(2047,round(1789773/((32 if triangle else 16)*440*2**((note-69)/12))-1)))
PHRASES={
'bloom':[72,76,79,76,74,77,81,77,72,76,79,83,81,79,76,-1,69,72,76,72,71,74,77,74,67,71,74,79,77,74,72,-1],
'starstring':[64,67,71,76,74,71,67,64,62,66,69,74,73,69,66,62,60,64,67,72,71,67,64,60,59,62,66,71,69,66,62,59],
'skythread':[69,72,76,81,79,76,72,69,67,71,74,79,77,74,71,67,65,69,72,77,76,72,69,65,64,67,71,76,74,71,67,-1],
'emberkeep':[57,60,64,-1,62,60,57,-1,55,59,62,-1,60,59,55,-1,53,57,60,-1,59,57,53,-1,52,55,59,-1,57,55,52,-1],
}
def music_source(game):
    notes=PHRASES[game];bass=[n-24 if n>=0 else -1 for n in notes]
    return nst_array('MELODY',[period(n) for n in notes],'u16')+nst_array('BASS',[period(n,True) for n in bass],'u16')

def png(path,w,h,rgb):
    def chunk(t,b):return struct.pack('>I',len(b))+t+b+struct.pack('>I',zlib.crc32(t+b)&0xffffffff)
    raw=b''.join(b'\0'+rgb[y*w*3:(y+1)*w*3] for y in range(h))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b''))

def validate_puzzles():
    # An independent line-candidate solver verifies that every clue set is unique.
    patterns=[format(i,'08b') for i in range(256)]
    for number,puzzle in enumerate(PUZZLES):
        rows=[[p for p in patterns if clues(p)==clues(row)]for row in puzzle]
        columns=[[p for p in patterns if clues(p)==clues(''.join(row[x]for row in puzzle))]for x in range(8)]
        board=[['?']*8 for _ in range(8)]
        for iteration in range(64):
            changes=0
            for vertical in (False,True):
                for line in range(8):
                    candidates=[p for p in (columns if vertical else rows)[line] if all(
                        board[i][line] in ('?',p[i]) if vertical else board[line][i] in ('?',p[i]) for i in range(8))]
                    assert candidates, f'Puzzle {number+1} has contradictory clues'
                    for i in range(8):
                        value=candidates[0][i]
                        if all(p[i]==value for p in candidates):
                            y,x=(i,line) if vertical else (line,i)
                            if board[y][x]=='?':board[y][x]=value;changes+=1
            if not changes:break
        assert [''.join(row)for row in board]==puzzle,f'Puzzle {number+1} must be solvable by line deductions without guessing'
        solutions=[]
        def search(board,candidates):
            y=len(board)
            if y==8:solutions.append(board);return
            for row in rows[y]:
                remaining=[[p for p in candidates[x]if p[y]==row[x]]for x in range(8)]
                if all(remaining):search(board+[row],remaining)
                if len(solutions)>=2:return
        search([],columns)
        assert len(solutions)==1 and solutions[0]==puzzle,f'Puzzle {number+1} must have a unique solution'

def generate():
    validate_puzzles()
    # All background/sprite transparent entries share the same universal backdrop.
    for game,palette in PALETTES.items():
        if game=='skythread':
            for i in range(0,32,4):palette[i]=0x0c
    for game in PALETTES:
        tiles=common_tiles(game);chr_data=b''.join(encode_tile(t) for t in tiles)
        (OUT/f'{game}.chr').write_bytes(chr_data*2)
        (OUT/f'{game}.pal').write_bytes(bytes(PALETTES[game]))
        (OUT/f'{game}.music.nst').write_text(music_source(game))
    (OUT/'bloom.map').write_bytes(nonogram().encode())
    (OUT/'starstring.map').write_bytes(rhythm().encode())
    (OUT/'emberkeep.map').write_bytes(dungeon().encode())
    screens=[];collision=[]
    for i,room in enumerate(ROOMS):
        encoded=mountain(room,i).encode();(OUT/f'skythread-{i}.map').write_bytes(encoded);screens.append(encoded)
        collision.extend({'#':1,'^':2,'E':3,'G':4}.get(c,0) for row in room for c in row)
    (OUT/'skythread.rooms.bin').write_bytes(b''.join(screens))
    (OUT/'skythread.collision.bin').write_bytes(bytes(collision))
    puzzles=[int(v) for puzzle in PUZZLES for line in puzzle for v in line]
    (OUT/'bloom.puzzles.bin').write_bytes(bytes(puzzles))
    # 8x8 rows/columns each get four right-aligned clue slots.
    all_clues=[]
    for puzzle in PUZZLES:
        for row in puzzle:all_clues.extend(([0]*4+clues(row))[-4:])
        for x in range(8):all_clues.extend(([0]*4+clues(''.join(row[x] for row in puzzle)))[-4:])
    (OUT/'bloom.clues.bin').write_bytes(bytes(all_clues))
    chart=[]
    for step in range(192):
        bar=step//8;beat=step%8
        lane=[0,2,1,3,0,1,2,3][(step+bar//4)%8]
        mask=1<<lane
        if beat in (3,7) and bar%3==0:mask=0
        if bar>=8 and beat in (0,4):mask|=1<<((lane+2)%4)
        if bar%4==3 and beat==0:mask|=128
        chart.append(mask)
    # Sustains last 24 frames; at expert tempo the next two events overlap.
    # Never require a fresh tap on a lane the player must keep held.
    for step,event in enumerate(chart):
        if event & 128:
            for future in range(step+1,min(step+3,len(chart))):chart[future] &= ~(event & 15)
    (OUT/'starstring.chart.bin').write_bytes(bytes(chart))
    manifest={'generator':'scripts/generate-assets.py','art':'Generated sprite atlas converted to three-color NES patterns, with authored tiles and layouts','music':'Four original 32-step compositions with distinct pulse and triangle arrangements','spriteTiles':{'cursor':140,'flower':144,'hero':148,'run':152,'gem':160,'slime':164,'bat':168,'chest':172,'gate':176,'spikes':180,'terrain':184,'note':192,'hold':196,'dash':208,'lantern':212,'stairs':216,'floor':220,'moss':224,'heart':232,'butterfly':240}}
    (OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    # Source-art contact sheet, distinct from the executing-ROM screenshots.
    colors=[(12,16,27),(32,42,59),(64,216,198),(255,235,180)]
    w=256;h=96;rgb=bytearray(w*h*3)
    tiles=common_tiles('skythread')
    for i,index in enumerate([140,144,148,152,156,160,164,168,172,176,180,184,188,192,196,200,208,212,216,220,224,232,240,248]):
        for yy in range(16):
            for xx in range(16):
                tile=tiles[index+(yy//8)*2+xx//8];color=colors[tile[yy%8][xx%8]]
                for sy in range(2):
                    for sx in range(2):
                        x=(i%8)*32+xx*2+sx;y=(i//8)*32+yy*2+sy;p=(y*w+x)*3;rgb[p:p+3]=bytes(color)
    png(ROOT/'docs'/'screenshots'/'original-sprites.png',w,h,bytes(rgb))
    print(f'Generated {len(list(OUT.iterdir()))} deterministic assets')
if __name__=='__main__':generate()
