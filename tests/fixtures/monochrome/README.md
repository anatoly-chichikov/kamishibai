# Monochrome gate production fixtures

These generated JPEGs exercise the production chroma gate. Checksums identify
the exact fixture bytes. Repository-level licensing is declared in the root
[LICENSE](../../../LICENSE) file.

| Fixture | Expected | SHA-256 |
| --- | --- | --- |
| `color-linger.jpg` | reject | `cd6b2bcac09b57459871f10d0337a9562d96179462b8ebc3ac98948682851707` |
| `color-water.jpg` | reject | `8aaa85f4b6e2955f12ab2a21ad60ace18e640b0f91321d534d99d42526382a97` |
| `mono-cut-through.jpg` | pass | `02924608f36da833993f1d8c28a87cbbe1711fd37eb6af1519e9fd69b0596c7e` |
| `mono-adversarial.jpg` | pass | `26bd4c51014ce690e60c45e01840015bcd7b00d1a556fc9492661f5aeab4da3e` |
| `mono-deed.jpg` | pass | `747d3b0e44f8213708698bad9ea5002ebafb6907b3a395e62c284273175bb9c7` |

The gate downsamples to a longest side of 256 pixels and measures
BT.601-style centered chroma. A pixel is chromatic above amplitude 7; the image
is rejected when at least 10% of pixels are chromatic. The two positive
fixtures measured roughly 48–54%, while the three confirmed monochrome
references measured 0%.
