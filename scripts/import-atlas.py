#!/usr/bin/env python3
"""Convert the generated transparent sprite atlas to reproducible NES pixel data.
ImageMagick is only needed to rerun this import, not for normal builds.
"""
from pathlib import Path
import subprocess,json
ROOT=Path(__file__).resolve().parents[1]
names=['mushroom','flower','butterfly','spirit','hero','run1','run2','gem','explorer','slime','bat','chest','lantern','terrain','gate','note']
colors=[(24,51,68),(100,184,155),(255,240,194)]
result={}
for i,name in enumerate(names):
    x=(i%4)*320;y=(i//4)*320
    data=subprocess.check_output(['magick',str(ROOT/'games/art/generated-atlas.png'),'-crop',f'320x320+{x}+{y}','+repage','-trim','-filter','point','-resize','16x16','-gravity','center','-background','none','-extent','16x16','-depth','8','rgba:-'])
    if name=='note':
        data=subprocess.check_output(['magick',str(ROOT/'games/art/generated-atlas.png'),'-crop',f'320x320+{x}+{y}','+repage','-trim','-filter','point','-resize','14x7!','-gravity','center','-background','none','-extent','16x8','-depth','8','rgba:-'])+bytes(16*8*4)
    pixels=[]
    for j in range(256):
        r,g,b,a=data[j*4:j*4+4]
        pixels.append(0 if a<100 else 1+min(range(3),key=lambda k:sum((v-c)**2 for v,c in zip((r,g,b),colors[k]))))
    result[name]=[''.join(str(v) for v in pixels[row*16:row*16+16])for row in range(16)]
(ROOT/'games/art/sprites.json').write_text(json.dumps(result,indent=2)+'\n')
decor={}
for i,name in [(0,'mushroom'),(1,'flower')]:
    data=subprocess.check_output(['magick',str(ROOT/'games/art/generated-atlas.png'),'-crop',f'320x320+{i*320}+0','+repage','-trim','-filter','point','-resize','30x30','-gravity','center','-background','none','-extent','32x32','-depth','8','rgba:-'])
    pixels=[]
    for j in range(1024):
        r,g,b,a=data[j*4:j*4+4]
        pixels.append(0 if a<100 else 1+min(range(3),key=lambda k:sum((v-c)**2 for v,c in zip((r,g,b),colors[k]))))
    decor[name]=[''.join(str(v) for v in pixels[row*32:row*32+32])for row in range(32)]
(ROOT/'games/art/decor.json').write_text(json.dumps(decor,indent=2)+'\n')
print('Imported generated sprites and 32-pixel garden decorations into NES patterns.')
