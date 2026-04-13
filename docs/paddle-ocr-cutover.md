# PaddleOCR Cutover

Этот переход заменяет системный `tesseract` на `ocr-rs` c `PP-OCRv5` моделями и MNN backend.

## Что меняется

- Runtime больше не зависит от системной установки `tesseract` и отдельных language packs
- OCR-модели доставляются лениво: первый OCR-backed запуск скачивает нужные `.mnn` и charset files в cache root приложения
- Legacy profile tokens остаются прежними, чтобы не ломать внешний контракт `profile.rs`
- Внутри runtime legacy tokens маршрутизируются в `PP-OCRv5` bundles

## Маршрутизация

- `eng` -> `en_PP-OCRv5_mobile_rec_infer.mnn`
- `eng+deu` -> `latin_PP-OCRv5_mobile_rec_infer.mnn`
- `eng+spa` -> `latin_PP-OCRv5_mobile_rec_infer.mnn`
- `eng+ell` -> `el_PP-OCRv5_mobile_rec_infer.mnn`
- `eng+rus` -> `cyrillic_PP-OCRv5_mobile_rec_infer.mnn`
- `eng+chi_sim` -> `PP-OCRv5_mobile_rec.mnn`
- shared detection for every route -> `PP-OCRv5_mobile_det.mnn`

## Почему так

- `ocr-rs` already wraps PaddleOCR + MNN and removes the old brew/apt dependency chain for end users
- The supported language bundles cover the current product set without changing `profile.rs`
- The lazy cache download keeps the repository small and the runtime self-contained from the user point of view

## Известный caveat

Detection model shared, recognition model language-specific. Для текущего runtime это нормально, потому что одна card image валидируется через target-language route. Если в одном изображении появятся смешанные Latin + Greek + Cyrillic + Chinese text boxes, следующая итерация должна делать multi-pass routing по box level, а не один recognizer на всю страницу.
