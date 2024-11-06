from dataclasses import dataclass
import cv2


@dataclass
class ImageWithExif:
    img: cv2.typing.MatLike
    focal_length: int
    pixel_x_dimension: int
    focal_plane_x_resolution: float

    def get_sensor_width_mm(self):
        return self.pixel_x_dimension / self.focal_plane_x_resolution * 25.4
