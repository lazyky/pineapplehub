import numpy as np

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
    return np.pi * r  ** 2