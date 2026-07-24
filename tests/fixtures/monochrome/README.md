# Monochrome gate production fixtures

These original provider JPEG bytes calibrate the production chroma gate. The
source experiments remain ignored; checksums make the tracked evidence
auditable.

| Fixture | Expected | Source | SHA-256 |
| --- | --- | --- | --- |
| `color-linger.jpg` | reject | campaign 001, `linger`, first image attempt | `cd6b2bcac09b57459871f10d0337a9562d96179462b8ebc3ac98948682851707` |
| `color-water.jpg` | reject | campaign 011, card `7ab6520dd6d3`, first image attempt | `8aaa85f4b6e2955f12ab2a21ad60ace18e640b0f91321d534d99d42526382a97` |
| `mono-cut-through.jpg` | pass | campaign 016, card `557f5e9e1f6e`, second image attempt | `02924608f36da833993f1d8c28a87cbbe1711fd37eb6af1519e9fd69b0596c7e` |
| `mono-adversarial.jpg` | pass | campaign 016, card `75d414c3e4e6`, first image attempt | `26bd4c51014ce690e60c45e01840015bcd7b00d1a556fc9492661f5aeab4da3e` |
| `mono-deed.jpg` | pass | campaign 016, card `a6bf5f1881ba`, first image attempt | `747d3b0e44f8213708698bad9ea5002ebafb6907b3a395e62c284273175bb9c7` |

The calibration downsamples to a longest side of 256 pixels and measures
BT.601-style centered chroma. A pixel is chromatic above amplitude 7; the image
is rejected when at least 10% of pixels are chromatic. The two positive
fixtures measured roughly 48–54%, while the three confirmed monochrome
references measured 0%.
