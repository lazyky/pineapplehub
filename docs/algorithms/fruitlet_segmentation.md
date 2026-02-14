# Pineapple Fruitlet Segmentation Pipeline

*Scale-Invariant Frequency Domain*

This document provides a comprehensive technical overview of the computer vision pipeline used in PineappleHub to count fruitlets on a pineapple. The pipeline is designed to be robust against lighting variations, fruit orientation, and distance by leveraging **Physical Scale Calibration** and **Frequency Domain Analysis**.

## Core Philosophy

The pineapple surface is mathematically modeled as a **Quasi-Periodic Signal** wrapped around a cylinder.

1.  **Physical Scale Invariance**: By detecting a physical reference (1 Yuan Coin, 25mm), we establish a `pixels_per_mm` scale. This allows all subsequent parameters (kernel sizes, thresholds, search windows) to be deterministically calculated, removing the need for fragile heuristics or manual tuning.
2.  **Skin vs. Flesh**: The skin is a high-contrast quasi-periodic texture (fruitlets separated by gaps). The flesh is low-frequency smooth texture.
3.  **Fruitlet Detection**: Fruitlets are "peaks" in a specific spatial frequency band determined by the physical scale.

---

## Algorithm Pipeline

### Step 1: Scale Calibration & Pre-processing

**Objective**: Establish physical scale, suppress noise, and derive morphological parameters.

1.  **Gaussian Smoothing**:
    Apply a Gaussian kernel ($\sigma = 1.0$) to remove sensor noise while preserving edge structure.
    $$ I_{smooth} = I_{raw} * G_\sigma $$

2.  **Scale Calibration (Coin Detection)**:
    Identify the reference object to establish the mapping from pixels to millimeters.
    *   **Detection**: Apply Otsu's thresholding and contour analysis.
    *   **Selection**: Identify the candidate with the **Highest Circularity** (> 0.85).
    *   **Derivation**:
        $$ \text{pixels\_per\_mm} = \frac{\text{Radius}_{coin\_px}}{12.5mm} $$
        (Assuming 1 Yuan Coin diameter = 25mm).

3.  **Parameter Derivation (CV-Based)**:
    All morphological and frequency parameters are derived from the physical scale:
    *   **Patch Size**: $3.0 \times R_{coin}$ (Approx 37.5mm).
        *   *Rationale*: Large enough to capture a full fruitlet (foreground) plus surrounding gaps (background) to ensure valid contrast calculation.
    *   **Adaptive Threshold Radius**: $1.0 \times R_{coin}$ (Approx 12.5mm).
        *   *Rationale*: Matches the structural scale of a half-fruitlet, filtering out internal texture details while preserving the overall mound shape.
    *   **Morphology Radius**: $0.15 \times R_{coin}$ (Approx 1.8mm).
        *   *Rationale*: Conservative size to close small specular highlights/gaps without merging adjacent fruitlets.
    *   **Contrast (Threshold C)**: $C = -0.5 \times \sigma_{global}$.
        *   *Rationale*: Dynamically adapts to global image contrast, ensuring only peaks significantly brighter than the local neighborhood are retained.

---

### Step 2: Adaptive Thresholding & ROI Extraction

**Objective**: Segmentation of the "Skin" surface using the deterministically derived parameters.

1.  **Adaptive Thresholding**:
    Use a local adaptive threshold (Bernsen/Mean) parameterized by the derived $R$ and Contrast $C$ (from patch variance).
    $$ B(x,y) = \begin{cases} 1 & \text{if } I(x,y) > \mu_{R}(x,y) + 0.5 \times \sigma_{global} \\ 0 & \text{otherwise} \end{cases} $$

2.  **Morphological Closing**:
    Fuse fragmented binary features using the derived radius.
    $$ B_{fused} = \text{Close}(B, R_{morph}) $$

3.  **Physical Area Filter**:
    *   Remove blobs where $\text{Area} < 0.2 \times \text{Area}_{coin}$.
    *   *Rationale*: Objects significantly smaller than a coin are physically too small to be valid fruitlets or skin patches, regardless of camera distance.

4.  **ROI Selection (Texture-Based)**:
    Distinguish between Skin (Target) and Flesh (Background).
    *   Iterate through top candidate regions (passed area filter).
    *   Calculate **Constrained Texture Score**:
        1.  Crop the candidate ROI.
        2.  Compute 2D FFT Spectrum.
        3.  **Frequency Masking**: Retain energy only within the "Expected Fruitlet Frequency Band" ($D \in [0.7 \times D_{target}, 1.3 \times D_{target}]$).
        4.  **Integration**: Sum the magnitude of energy within the mask and normalize by ROI area.
        $$ S = \frac{1}{Area} \sum_{(u,v) \in Ring} |\mathcal{F}(u, v)| $$
    *   **Selection**: The region with the highest Score $S$ is selected as the Skin ROI.

    6.  **Spatially Adaptive Filtering (Step 2b)**:
        Instead of relying on Cylindrical Unwrapping or Global Frequency Locking, we enhance fruitlet signals directly on the image using spatially variant filters based on the pineapple's physical curvature.

        *   **Generalized Ellipsoid Model**:
            *   **Problem**: The pineapple is approximately an ellipsoid. Due to perspective projection, surface normals deviate further from the camera axis towards the edges, causing texture foreshortening in the radial direction.
            *   **Modeling**:
                Based on the ROI ($W \times H$), calculate the normalized radial distance $r$:
                $$ u = \frac{2(x - c_x)}{W}, \quad v = \frac{2(y - c_y)}{H} $$
                $$ r = \sqrt{u^2 + v^2}, \quad r \in [0, 1] $$
                The foreshortening factor $k$ decreases as $r$ increases (edges are flatter):
                $$ k(r) = \cos(\arcsin(r)) = \sqrt{1 - r^2} $$
                (Set $k_{min} \approx 0.3$ to prevent numerical instability at edges).

        *   **Multi-Scale Competition**:
            *   **Adaptive Kernels**: At position $(x, y)$, construct a set of **Rotated Elliptical Laplacian/Gaussian Kernels**.
                *   **Target Feature**: **Dark Floral Cavities**. We no longer detect the entire fruitlet mound.
                *   **Minor Axis (Radial)**: $\sigma \times k(r)$ (Matches physical foreshortening).
                *   **Major Axis (Tangential)**: $\sigma$ (Constant, no foreshortening).
                *   **Feature Scale**: $\sigma \approx 2.0mm$ (Matches cavity radius).
            *   **Competition Mechanism**: To handle irregularities (pineapple is not a perfect ellipsoid), we don't apply just the single theoretical kernel $k(r)$. Instead, we concurrently compute responses for a set of scales:
                $$ K \in \{ k(r), 1.2k(r), 0.8k(r) \} $$
                $$ R(x, y) = \max_{K} ( I(x, y) * G_K ) $$
            *   **Result**: The response peaks only when the kernel scale matches the actual local physical scale of the texture. This allows the data to "choose" the best scale, naturally adapting to curvature variations at the top, bottom, and sides, achieving omni-directional correction.

---

### Step 3: Maxima Finding & Counting

**Objective**: Extract fruitlet positions from the enhanced response map.

1.  **Illumination & Background Suppression**:
    Since bandpass kernels (like LoG or DoG) are used, low-frequency illumination components are automatically removed. The response map is near zero in flat background areas.

2.  **Local Maxima Finding**:
    *   **Dynamic Thresholding**:
        Use a relative threshold to adapt to overall signal strength:
        Use a relative threshold to adapt to overall signal strength:
        $$ T = 0.5 \times \max(R_{scale\_map}) $$
        Peaks below $T$ are ignored.
        Peaks below $T$ are ignored.
    *   **Physical NMS (Non-Maximum Suppression)**:
        Suppress neighbor peaks within a radius of $1.0 \times R_{coin}$ (approx 12.5mm).
        *Rationale*: Updated based on feedback. Prevents double counting of single fruitlets.

## Advantages

*   **Omni-Directional Robustness**: The generalized model handles both vertical and horizontal perspective foreshortening, more accurate than a single-axis cylindrical model.
*   **Shape Adaptation**: The multi-scale competition mechanism "lets the data speak," locking onto features via local contrast maximization rather than enforcing a perfect geometric shape.
*   **Efficiency**: Removes complex IFFT and geometric interpolation, operating directly in the image domain.
*   **Scale Invariance**: Remains fully grounded in the physical coin calibration.
