# Production topology fixtures

These generated images and scene descriptions exercise crossing, open-frame,
slanted-rail, and staggered-grid geometry in the production topology gate.
Repository-level licensing is declared in the root [LICENSE](../../../LICENSE)
file.

The crossing and open-frame JPEGs retain Google's embedded C2PA metadata.

| Fixture | Geometry exercised | SHA-256 |
| --- | --- | --- |
| `crossing-exact.jpg` | Crossing preserves three closed regions | `da7742fc6366af4fe7b487abfce5f74591b699f7b414abe730ab442787460b12` |
| `crossing-exact.scene.json` | Scene declaring the crossing layout | `75a2cc7782244543be664d50d97cc9e81989295aabc68e1a45496c6c608ad93d` |
| `crossing-merged.jpg` | Crossing merges the declared pair of regions | `d8707f89d39d17a9f15d43f4d798ce6782c06fb1fe62df27947572f74ee58803` |
| `crossing-merged.scene.json` | Scene declaring the merged crossing pair | `8e30c8e7dff918496e92eb22fc156847bc9ed82348db7f04845e0b9637ffef94` |
| `open-frame.jpg` | Closed panel must not satisfy an open-frame declaration | `f8a79088fc50d9fa21da370b69264b5744659e5acd9baa80e151bd6be9472e90` |
| `open-frame.scene.json` | Scene declaring an open frame | `b41c6ed4102c1b634acf1c2c1e3ca84db43624524f823764649c1ea1177a8d33` |
| `slanted-rail-shifted.jpg` | Three oblique regions with a shifted divider | `b745ac99c1f51f91bc0ab4e55124349feb70a34aeb06af9eaa71cae73a5220d9` |
| `staggered-grid-shifted.jpg` | Shifted staggered dividers around isolated centers | `6158f5345f2ec2f34c6b45ad433102c26ad1031e42487d40afc0c21e3a491848` |

The scene JSON files and [tests/scene.rs](../../scene.rs) define the expected
geometry. SHA-256 values identify the tracked files directly.
