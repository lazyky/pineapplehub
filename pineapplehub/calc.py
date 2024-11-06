import numpy as np
import cv2
from scipy.integrate import quad


def distance_to_major_axis(point, center, angle):
    """Calculate the distance between the point and the major axis of the ecllipse.

    Args:
        point: the coordinate `(x, y)` of the point
        center: the center coordinate `(x, y)` of the ecllipse
        angle (): the rotation angle for ecllipse.

    Returns:
        the distance between the point and the center
    """
    vector = np.array([point[0] - center[0], point[1] - center[1]])
    direction_angle = np.arctan2(vector[1], vector[0])
    r = np.sqrt((vector[0] ** 2) + (vector[1] ** 2))
    adjusted_angle = direction_angle - angle
    distance = abs(r * np.cos(adjusted_angle))

    return distance


def calc_area(r: float):
    """For integral: to calculate the area of the current layer.

    Args:
        r (float): the radius

    Returns:
        area of the layer
    """
    return np.pi * r**2


def connect_contours(contours):
    simplified_contours = []
    for contour in contours:
        epsilon = 0.005 * cv2.arcLength(contour, True)
        approximated = cv2.approxPolyDP(contour, epsilon, True)
        simplified_contours.append(approximated)

    return simplified_contours


def detect_circle(contours):
    circles = []
    for contour in connect_contours(contours):
        area = cv2.contourArea(contour)
        x, y, w, h = cv2.boundingRect(contour)
        aspect_ratio = float(w) / h

        if 0.95 < aspect_ratio < 1.05:
            perimeter = cv2.arcLength(contour, False)
            # Roundness: https://diplib.org/diplib-docs/features.html#shape_features_Roundness
            circularity = 4 * np.pi * area / perimeter**2

            if circularity > 0.9:
                diameter = int(np.sqrt(4 * area / np.pi))
                circles.append((contour, diameter))

    return circles


def remove_hypotenuse(contours):
    filtered_contours = []

    for contour in contours:
        area = cv2.contourArea(contour)

        # 忽略面积太小的轮廓
        if area < 100:
            continue

        x, y, w, h = cv2.boundingRect(contour)
        aspect_ratio = float(w) / h

        # 过滤宽高比大于阈值的轮廓
        if 0.2 < aspect_ratio < 5:
            filtered_contours.append(contour)

    return filtered_contours


def get_new_width(
    focal_length,
    real_width,
    image_width,
    object_pixel_width,
    sensor_width,
    pixels_moved,
):
    initial_distance_px = (
        focal_length
        * real_width
        * image_width
        * image_width
        / (object_pixel_width * sensor_width * sensor_width)
    )
    new_distance_mm = (initial_distance_px - pixels_moved) / image_width * sensor_width
    new_pixel_width = (
        focal_length * real_width * image_width / new_distance_mm / sensor_width
    )

    return new_pixel_width


def calc_volume(distances):
    return np.sum(
        np.array(
            [
                quad(calc_area, distances[i], distances[i + 1])[0]
                for i in range(len(distances) - 1)
            ]
        )
    )
