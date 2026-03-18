# Design Assets

## Favicon

### Tilted Pixel Art Version (Current)

**Generation prompt:**
> A high-quality pixel art icon. Subject: A stylized pineapple in pixel art style, tilted about 20 degrees to the right, giving it a dynamic playful look. It should NOT be perfectly upright. Style: 16-bit vibrant pixel art. Include subtle debug overlay: thin cyan/neon-blue crosshairs through the center and small bounding box corner brackets in cyan. Colors: Rich golden-orange fruit body, dark green crown leaves. NO TEXT of any kind — no labels, no words, no letters, no numbers. Background: SOLID PURE MAGENTA (#FF00FF) everywhere with no gradients or patterns. The pineapple should be centered in the frame.

**Post-processing** (required — the generator cannot produce true transparency):

```python
from PIL import Image
import numpy as np

img = Image.open("generated.png").convert("RGBA")
data = np.array(img)
r, g, b = data[:,:,0].astype(int), data[:,:,1].astype(int), data[:,:,2].astype(int)

# Chroma-key: remove solid magenta
magenta = (r > 180) & (g < 100) & (b > 180)
data[magenta, 3] = 0

# Soften anti-aliased edges
for y in range(data.shape[0]):
    for x in range(data.shape[1]):
        rv, gv, bv = int(data[y,x,0]), int(data[y,x,1]), int(data[y,x,2])
        if rv > 150 and bv > 150 and gv < 120 and data[y,x,3] > 0:
            strength = min(rv, bv) - gv
            if strength > 100:
                data[y,x,3] = max(0, 255 - int(strength * 1.5))

Image.fromarray(data).save("assets/favicon.png")
```

**Generated File:** `assets/favicon.png`
