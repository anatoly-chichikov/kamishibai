# Production topology fixtures

These fixtures are preserved Gemini image attempts from ignored local experiment
archives. They exercise real crossing, open-frame, slanted-rail, and staggered-grid
pages that the production topology gate previously rejected.

The Dutch campaign used `gemini-3.1-flash-image-preview`; that model is recoverable
from its cost sidecars, and its three JPEGs retain Google's C2PA provenance manifest.
That archive does not bind the rejected candidates to one final session id, so no
more specific session claim is made here. The two campaign 008 JPEGs were produced
by `gemini-3.1-flash-image` according to their picture cost sidecars and were
re-encoded as described below. Repository-level licensing is declared in the root
`LICENSE` file.

| Fixture | Archived source | Term | Attempt | Archived verdict | SHA-256 |
| --- | --- | --- | ---: | --- | --- |
| `crossing-exact.jpg` | `rejected-candidates/uitwaaien/attempts/attempt-0011.jpg` | `uitwaaien` | 11 | rejected: registered panel topology was not detected | `da7742fc6366af4fe7b487abfce5f74591b699f7b414abe730ab442787460b12` |
| `crossing-exact.scene.json` | `rejected-candidates/uitwaaien/scene.json` | `uitwaaien` | 11 | source scene | `75a2cc7782244543be664d50d97cc9e81989295aabc68e1a45496c6c608ad93d` |
| `crossing-merged.jpg` | `rejected-candidates/achtervolgen/attempts/attempt-0019.jpg` | `achtervolgen` | 19 | rejected: registered panel topology was not detected | `d8707f89d39d17a9f15d43f4d798ce6782c06fb1fe62df27947572f74ee58803` |
| `crossing-merged.scene.json` | `rejected-candidates/achtervolgen/scene.json` | `achtervolgen` | 19 | source scene | `8e30c8e7dff918496e92eb22fc156847bc9ed82348db7f04845e0b9637ffef94` |
| `open-frame.jpg` | `rejected-candidates/paraplu/attempts/attempt-0001.jpg` | `paraplu` | 1 | rejected: registered panel topology was not detected | `f8a79088fc50d9fa21da370b69264b5744659e5acd9baa80e151bd6be9472e90` |
| `open-frame.scene.json` | `rejected-candidates/paraplu/scene.json` | `paraplu` | 1 | source scene | `b41c6ed4102c1b634acf1c2c1e3ca84db43624524f823764649c1ea1177a8d33` |
| `slanted-rail-shifted.jpg` | PR #41 campaign 008, card `a29a61d8a670`, attempt 1 | `piled up` | 1 | rejected: registered panel topology was not detected | `b745ac99c1f51f91bc0ab4e55124349feb70a34aeb06af9eaa71cae73a5220d9` |
| `staggered-grid-shifted.jpg` | PR #41 campaign 008, card `038c6042b87e`, attempt 2 | `get my brother to help` | 2 | rejected: registered panel topology was not detected | `6158f5345f2ec2f34c6b45ad433102c26ad1031e42487d40afc0c21e3a491848` |

The first six source paths are relative to the archived Dutch campaign directory,
and those copied files are byte-identical to the archive entries. The two campaign
008 fixtures keep their original 1024 by 1024 dimensions and were re-encoded at JPEG
quality 50 to keep the regression corpus compact; their original SHA-256 values are
`416216c0530be0ff564697f6f4ddfbec8077cb01c63260c0b8355286ccfdefa2` and
`aeae23383d856983fe6b4276bc70693a2ad32c98e59d4828d4a3bb8d054f62d3`.
Their canonical geometry is declared directly in `tests/scene.rs`.
