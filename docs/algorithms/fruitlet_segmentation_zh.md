# 菠萝果眼分割算法流程

*尺度不变频域分析 (Scale-Invariant Frequency Domain)*

本文档详细介绍了 PineappleHub 用于菠萝果眼计数的计算机视觉流水线。该流程利用 **物理尺度校准** 和 **频域分析**，旨在实现对光照变化、果实因距离产生的尺度变化以及不同拍摄角度的鲁棒性。

## 核心理念 (Core Philosophy)

菠萝表面被数学建模为包裹在圆柱体上的 **准周期信号 (Quasi-Periodic Signal)**。

1.  **物理尺度不变性 (Physical Scale Invariance)**：通过检测物理参照物（1元硬币，25mm），我们建立了 `pixels_per_mm`（像素/毫米）比例尺。这使得所有后续参数（核大小、阈值、搜索窗口）都可以被确定性地计算出来，从而消除了脆弱的经验法则或手动调参的需求。
2.  **果皮与果肉 (Skin vs. Flesh)**：果皮是高对比度的准周期纹理（果眼被间隙分隔）。果肉则是低频的光滑纹理。
3.  **果眼检测 (Fruitlet Detection)**：果眼被视为由物理尺度确定的特定空间频率带中的“波峰”。

---

## 算法流程 (Algorithm Pipeline)

### 第一步：尺度校准与预处理 (Step 1: Scale Calibration & Pre-processing)

**目标**：建立物理比例尺，抑制噪声，并推导形态学参数。

1.  **高斯平滑 (Gaussian Smoothing)**：
    应用高斯核 ($\sigma = 1.0$) 以去除传感器噪声，同时保留边缘结构。
    $$ I_{smooth} = I_{raw} * G_\sigma $$

2.  **尺度校准 (Scale Calibration)**：
    识别参照物以建立像素到毫米的映射。
    *   **检测**：应用 Otsu 阈值化和轮廓分析。
    *   **选择**：筛选出 **圆度最高 (Highest Circularity)** (> 0.85) 的候选对象。
    *   **推导**：
        $$ \text{pixels\_per\_mm} = \frac{\text{Radius}_{coin\_px}}{12.5mm} $$
        (假定1元硬币直径 = 25mm)。

3.  **基于CV理论的参数推导 (Parameter Derivation)**：
    所有形态学和频域参数均由物理比例尺推导而来：
    *   **Patch Size (局部窗口大小)**: $3.0 \times R_{coin}$ (约 37.5mm)。
        *   *依据*: 足够大以包含一个完整的果眼（前景）加上周围的间隙（背景），确保有效的对比度计算。
    *   **Adaptive Threshold Radius (自适应阈值半径)**: $1.0 \times R_{coin}$ (约 12.5mm)。
        *   *依据*: 匹配半个果眼的结构尺度，过滤掉内部纹理细节的同时保留整体隆起的形状。
    *   **Morphology Radius (形态学半径)**: $0.15 \times R_{coin}$ (约 1.8mm)。
        *   *依据*: 保守的尺寸，用于闭合细小的反光或间隙，同时避免合并相邻的果眼。
    *   **Contrast (对比度阈值)**: $C = -0.5 \times \sigma_{global}$。
        *   *依据*: 动态适应图像的整体对比度，确保只保留显著亮于局部的峰值。

---

### 第二步：自适应二值化与ROI提取 (Step 2: Adaptive Thresholding & ROI Extraction)

**目标**：使用确定性推导的参数分割“果皮”表面。

1.  **自适应二值化 (Adaptive Thresholding)**：
    使用局部自适应阈值（Bernsen/Mean），参数为推导出的 $R$ 和对比度 $C$。
    $$ B(x,y) = \begin{cases} 1 & \text{if } I(x,y) > \mu_{R}(x,y) + 0.5 \times \sigma_{global} \\ 0 & \text{otherwise} \end{cases} $$

2.  **形态学闭运算 (Morphological Closing)**：
    使用推导出的半径融合破碎的二值特征。
    $$ B_{fused} = \text{Close}(B, R_{morph}) $$

3.  **物理面积过滤 (Physical Area Filter)**：
    *   移除 $\text{Area} < 0.2 \times \text{Area}_{coin}$ 的斑块。
    *   *依据*: 显著小于硬币的物体在物理上太小，不可能是有效的果眼或果皮块，无论拍摄距离如何。

4.  **ROI 选择 (基于纹理评分)**:
    区分果皮（目标）和果肉（背景）。
    *   对于通过面积过滤的候选区域（Top Candidates）。
    *   计算 **受限纹理得分 (Constrained Texture Score)**：
        1.  裁剪出候选 ROI 区域。
        2.  计算 2D FFT 频谱。
        3.  **频率掩膜 (Frequency Masking)**：仅保留在“果眼预期频率环”内的能量（$D \in [0.7 \times D_{target}, 1.3 \times D_{target}]$）。
        4.  **积分求和**：计算掩膜区域内的总能量幅值，并除以区域像素总数进行归一化。
        $$ S = \frac{1}{Area} \sum_{(u,v) \in Ring} |\mathcal{F}(u, v)| $$
    *   **选择**: 得分 $S$ 最高的区域被识别为果皮 ROI。

    6.  **空域自适应滤波 (Step 2b: Spatially Adaptive Filtering)**:
        不再依赖圆柱面展开或全局频率锁定，而是根据菠萝的物理曲率，使用随位置变化的滤波器直接在图像上增强果眼信号。

        *   **广义椭球透视模型 (Generalized Ellipsoid Model)**:
            *   **问题**: 菠萝近似为椭球体。由于透视投影，表面法线偏离相机光轴越远，纹理在径向上的收缩越严重。
            *   **建模**:
                基于 ROI（$W \times H$），计算归一化径向距离 $r$：
                $$ u = \frac{2(x - c_x)}{W}, \quad v = \frac{2(y - c_y)}{H} $$
                $$ r = \sqrt{u^2 + v^2}, \quad r \in [0, 1] $$
                透视收缩因子 $k$ 随着 $r$ 的增加而减小（边缘处变扁）：
                $$ k(r) = \cos(\arcsin(r)) = \sqrt{1 - r^2} $$
                (设置 $k_{min} \approx 0.3$ 以防止边缘数值不稳定)。

        *   **多尺度竞争滤波 (Multi-Scale Competition)**:
            *   **自适应核**: 在位置 $(x, y)$，构建一组 **旋转椭圆拉普拉斯/高斯核 (Rotated Elliptical Kernels)**。
                *   **目标特征**: **深色花腔 (Dark Floral Cavities)**。不再检测整个果眼隆起。
                *   **短轴 (径向)**: $\sigma \times k(r)$ (匹配物理收缩)。
                *   **长轴 (切向)**: $\sigma$ (保持不变，无透视收缩)。
                *   **特征尺度**: $\sigma \approx 2.0mm$ (匹配花腔半径)。
            *   **竞争机制**: 为了应对菠萝形状的不规则（非完美椭球），在每个像素点并非只应用唯一的理论核 $k(r)$，而是并发计算一组尺度的响应：
                $$ K \in \{ k(r), 1.2k(r), 0.8k(r) \} $$
                $$ R(x, y) = \max_{K} ( I(x, y) * G_K ) $$
            *   **结果**: 仅当核的尺度与局部纹理的实际物理尺度最匹配时，响应达到峰值。这让数据自己决定最佳尺度，从而自然地适应了果实顶端、底端和侧面的曲率变化，实现全向校正。

---

### 第三步：极值查找与计数 (Step 3: Maxima Finding & Counting)

**目标**：从增强后的响应图中提取果眼位置。

1.  **光照与背景抑制**:
    由于使用了带通性质的卷积核（如 LoG 或 DoG），光照低频分量已被自动移除，且响应图在平坦背景处接近零。

2.  **局部极大值查找 (Local Maxima Finding)**：
    *   **动态阈值 (Dynamic Thresholding)**：
        使用相对阈值以适应整体信号强度：
        使用相对阈值以适应整体信号强度：
        $$ T = 0.5 \times \max(R_{scale\_map}) $$
        低于 $T$ 的峰值被忽略。
        低于 $T$ 的峰值被忽略。
    *   **物理非极大值抑制 (Physical NMS)**：
        抑制半径 $1.0 \times R_{coin}$ (约 12.5mm) 内的邻近峰值。
        *依据*: 根据用户反馈更新，防止单果眼重复计数。

## 优势

*   **全向几何鲁棒性**: 广义模型同时解决了垂直和水平方向的透视收缩，比单一圆柱模型更准确。
*   **形状自适应**: 多尺度竞争机制“让数据说话”，像生物视觉一样通过局部对比度最大化来锁定特征，而不强求果实形状完美。
*   **高效性**: 移除了复杂的 IFFT 和几何变换插值，直接在原图域操作，计算更加直接。
*   **尺度不变性**: 依然完全基于物理硬币校准。
