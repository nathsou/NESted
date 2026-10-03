# Generated sprite source

`generated-atlas.png` was made with image generation for this project: a 4×4
transparent pixel-art atlas in navy, sage and cream, containing mushroom,
flower, butterfly, garden spirit, climber/run frames, crystal, explorer,
slime, bat, chest, lantern, moss terrain, door and rhythm note.

`scripts/import-atlas.py` extracts each cell, fits it to 16×16 pixels, quantizes
it to transparent plus three palette indices, and saves `sprites.json`.
Mushroom and flower decorations also get 32×32 data in `decor.json`.
ImageMagick is needed only when repeating that conversion.

`scripts/generate-assets.py` combines these patterns with authored font, UI,
collision, cell, spike, floor, sustain and effect tiles and produces real NES
2-bpp CHR. Colors are assigned by the game's hardware palettes. Generated
source art is therefore adapted to NES limits rather than loaded as a browser
image. Game screenshots come from emulator PPU output.
