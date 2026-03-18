# PineappleHub

<p align="center">
  <a href="README_zh.md">中文</a> | English
</p>

<p align="center">
  <a href="https://github.com/TT-Industry/pineapplehub/actions/workflows/deploy.yml"><img src="https://github.com/TT-Industry/pineapplehub/actions/workflows/deploy.yml/badge.svg" alt="Deploy"></a>
  <a href="https://tt-industry.github.io/pineapplehub"><img src="https://img.shields.io/badge/demo-live-brightgreen?logo=github" alt="Live Demo"></a>
  <img src="https://img.shields.io/badge/rust-nightly--2025--09--23-orange?logo=rust" alt="Rust Nightly">
  <img src="https://img.shields.io/badge/target-wasm32-blueviolet?logo=webassembly" alt="WebAssembly">
  <img src="https://img.shields.io/badge/unsafe-denied-success" alt="No Unsafe">
  <img src="https://img.shields.io/badge/clippy-pedantic-informational" alt="Clippy Pedantic">
  <img src="https://img.shields.io/badge/100%25-Rust-dea584?logo=rust" alt="Pure Rust">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-BSL--1.1-EE5A24" alt="License: BSL 1.1"></a>
</p>

> _Why 99 years? — A nod to a certain lease agreement in Far East history._

A browser-based pineapple fruit quality measurement tool built with Rust + WebAssembly.

Upload a photo of a bisected pineapple with a 1 Yuan coin for scale, and PineappleHub automatically measures fruit geometry and fruitlet eye count.

## Features

- **Automatic Scale Calibration** — Detects the 1 Yuan coin (Ø 25 mm) in the photo to establish pixel-to-millimetre mapping; no manual calibration needed.
- **Fruit Geometry Measurement** — Automatically measures fruit height, width, volume, and surface area.
- **Fruitlet Eye Sizing** — Measures the representative fruitlet eye at the equator.
- **Whole-Fruit Eye Count** — Estimates the total number of fruitlet eyes on the entire fruit.

## Known Issues

- **Uniformly pale fruit** — On fruit with very light, uniform colouring, the fine wrinkles *inside* a fruitlet eye and the grooves *between* adjacent eyes may have identical width, depth, and contrast. This makes it impossible for any brightness-based segmentation to reliably isolate a single fruitlet eye: results are either fragmented (one eye split into multiple pieces) or merged (multiple neighbouring eyes counted as one). Measurements for such fruit may carry larger error; when processed following the [Best Practice](#best-practice), they can generally be automatically flagged as *suspect*.

## Best Practice

- **Batch visually similar fruit together** — Group fruit of similar size, shape, and colour in each batch run. The IQR-based statistical outlier detection works best when the batch is homogeneous, allowing it to more reliably highlight fruit with potentially inaccurate measurements for manual review.

## Documentation

- [Algorithm Documentation](docs/algorithms/algorithm.md)
- [Debug Image Interpretation](docs/user_guide/debug_interpretation.md)
