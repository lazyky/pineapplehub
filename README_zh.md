<img align="right" src="assets/favicon.png" alt="logo" width="128">

# PineappleHub

<p align="center">
  中文 | <a href="README.md">English</a>
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


基于 Rust + WebAssembly 的浏览器端菠萝果实品质测量工具。

上传一张带有 1 元硬币（用作比例尺）的菠萝剖面照片，PineappleHub 即可自动测量果实几何参数与小果眼数量。

## 功能特性

- **自动比例标定** — 自动识别照片中的 1 元硬币（Ø 25 mm），建立像素到毫米的映射，无需手动标定。
- **果实几何测量** — 自动测量果实的高度、宽度、体积和表面积。
- **小果眼测量** — 测量赤道位置的代表性小果眼尺寸。
- **全果眼数估算** — 估算整颗果实的小果眼总数。

## 已知局限

- **通体浅色果实** — 对于颜色均匀偏浅的果实，小果眼*内部*的褶皱与相邻小果眼*之间*的沟壑可能具有相同的宽度、深度和对比度，导致任何基于亮度的分割方法都无法可靠地隔离单个小果眼：结果要么被切碎（果眼被一分为多），要么合并（多个相邻果眼被视为一个）。此类果实的测量可能偏差较大；按照[最佳实践](#最佳实践)进行批处理时，一般能被自动标记为*可疑*。

## 最佳实践

- **将外观相近的果实放在同一批次** — 每次批处理时，尽量将大小、形状、颜色相近的果实归为一组。基于 IQR 的统计离群值检测在批次同质性较高时效果更好，能更可靠地标出可能测量不准确的果实以供人工复核。

## 文档

- [算法文档](docs/algorithms/algorithm_zh.md)
- [调试图解读](docs/user_guide/debug_interpretation_zh.md)

## 许可证

本项目采用 [Business Source License 1.1](LICENSE) 许可。

- **严格禁止商业用途。** Additional Use Grant 设定为 *None*——在未与许可方另行签订商业协议的情况下，禁止将本软件用于任何生产环境。
- **允许非生产用途。** 你可以自由地复制、修改和再分发代码，用于个人学习、学术研究或内部评估。
- **变更日期：2125-03-18。** 届时许可将自动转换为 [GNU Affero 通用公共许可证 v3.0](https://www.gnu.org/licenses/agpl-3.0.html)（或更高版本），本应用将成为完全开源软件。

> _为什么是 99 年？—— 当年签 99 年，是笃定届时无人会来追究。此处亦然。_
