# Debug Visualization Guide

This guide explains how to interpret the intermediate visualizations shown in the PineappleHub debug UI. Understanding these outputs is crucial for diagnosing issues with lighting, focus, or algorithm parameters.

---

## 1. Smoothing
**What it is:** The original image after applying a Gaussian Blur ($\sigma=1.0$).
*   **Normal:** Slightly blurry but edges of fruitlets remain visible. Sensor noise is suppressed.
*   **Debug:** If this looks too blurry, the original image might be out of focus. If too noisy, the camera ISO might be too high.

## 2. Scale Calibration
**What it is:** Visualizes the detected physical reference (coin).
*   **Normal:** The coin is highlighted with a red box.
*   **Debug:** If the coin is not detected (no red box), all subsequent adaptive parameters will fail. Check for occlusion or glare on the coin.

## 3. Texture Patch (Adaptive Thresholding)
**What it is:** The result of applying the calculated parameters (Radius $R$, Contrast $C$) to the whole image.
*   **Normal:** A noisy "star map". Fruitlet centers appear as white blobs. Gaps appear black.
*   **Debug:**
    *   **All White**: Contrast threshold $C$ too low (or zero).
    *   **All Black**: Contrast threshold $C$ too high.

## 4. Binary (Morphological Closing)
**What it is:** The "Star Map" from Step 3 is fused. Small dots merge into solid blobs.
*   **Normal:** Distinct, roughly circular blobs representing individual fruitlets.
*   **Debug:** If blobs are fused into giant continents, the estimated Radius $R$ is too large. If blobs are shattered, $R$ is too small.

## 5. Morphology / ROI Extraction
**What it is:** The final "Skin Region" cropped from the image.
1.  **Rotated ROI**: The original ROI, rotated to align the pineapple axis vertically. Due to **Spatially Adaptive Filtering**, we no longer need cylindrical unwrapping.

*   **Normal:**
    *   The pineapple should be upright.
    *   The image should compactly contain most of the fruitlet area.
*   **Debug:**
    *   **Tilted**: If the image is still tilted, `MinAreaRect` failed to find the correct orientation.
    *   **Empty**: If the crop is empty, texture scoring failed.

## 6. Adaptive Response Map
**What it is:**Meaning:** Enhanced image processed with **Spatial Adaptive Filtering (LoG Kernel)**.
*   **Note:** This is **NOT** a crop of the original photo. It is a **calculated Response Map**. Brightness indicates the algorithm's confidence that a pixel is a fruitlet center.
*   **Visuals:** Background (flesh) should be suppressed to black. Fruitlet centers (usually dark floral cavities) should appear as **bright white spots**. The algorithm now targets "Dark Spots" instead of "Bright Mounds" to avoid highlights.
*   **How to Interpret:**
    *   **Good**: Clear, separated bright spots, like stars in the night sky. Each spot corresponds to a single fruitlet.
    *   **Bad (Under-segmentation/Merged)**: Spots look like "worms", "stripes", or "brain coral". This means the filter scale `Sigma` is **too large**, causing adjacent fruitlets to merge.
    *   **Bad (Over-segmentation/Fragmented)**: Spots are tiny, broken, or reacting to noise. This means the filter scale `Sigma` is **too small**.
    *   **All Black**: Illumination Correction (High-Pass) removed the signal, or Sigma estimate is too large.

## 7. Frequency Analysis (Final Count)
**What it is:** The final tally. Red crosses mark detected fruitlets.
*   **Debug:**
    *   **Double Counting**: Two crosses on one eye. This is usually solved by **Physical NMS** (Radius 12.5mm). If it occurs, the coin size estimation is likely too small.
    *   **Missed Counting**: Clear fruitlets have no cross. **Dynamic Threshold** ($0.5 \times Max$) might be too high, or the response map (Step 6) has insufficient contrast.
