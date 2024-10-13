#! /usr/bin/env python

from multiprocessing import Manager, Queue
from pineapplehub.calc import distance_to_major_axis, calc_area
import cv2
import numpy as np
from nicegui import run, ui
from PIL import Image
import math
from scipy.integrate import quad

ui.add_head_html(
    """
    <script>
    function emitSize() {
        emitEvent('resize', {
            width: document.body.offsetWidth,
            height: document.body.offsetHeight,
        });
    }

    window.onload = emitSize; 
    window.onresize = emitSize;
    </script>
"""
)


def resize(e):
    global screen_w, screen_h

    screen_w = e.args.get("width")
    screen_h = e.args.get("height")


ui.on("resize", lambda e: resize(e))


def zoom_in(img):
    zoomed_img.set_source(img)
    dialog.open()


stepper_imgs = []


def render_steppers(q: Queue):
    """It's the best way to update steps since:

    - Binding propagation of pre-defined images can be very expensive
    - We cannot put the whole `ui.step()` within `ui.teleport()`

    Args:
        q (Queue): The queue for communication between the main and the heavy computation process
    """
    ctx = q.get()

    if ctx is None:
        stepper.next()
    else:
        with ui.teleport(f"#c{stepper.id} > div:nth-child(1) .q-stepper__step-inner"):
            # Cannot use `ui.interactive_image()` here
            stepper_imgs.append(
                ui.image(ctx).on("click", lambda: zoom_in(ctx)).classes("w-64")
            )


def resize_img(arr, to_rgb=False) -> Image:
    """Resize image:
    The longer size of the image will be a half of the shorter size of the screen.

    Args:
        arr: the image in numpy.ndarray form
        to_rgb: convert the image to RGB mode (only need when it's BGR)

    Returns:
        img: resized image
    """
    if to_rgb:
        arr = cv2.cvtColor(arr, cv2.COLOR_BGR2RGB)
    img = Image.fromarray(arr)
    w, h = img.size

    screen_shorter = min(screen_w, screen_h)

    if w > h:
        new_w = screen_shorter
        new_h = int((new_w / w) * h)
    else:
        new_h = screen_shorter
        new_w = int((new_h / h) * w)

    return img.resize((new_w, new_h), Image.Resampling.LANCZOS)


def compute(img, q):
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    q.put(resize_img(gray))
    q.put(None)

    smoothed = cv2.GaussianBlur(gray, (3, 3), 0)
    q.put(resize_img(smoothed))
    q.put(None)

    circles = cv2.HoughCircles(
        smoothed,
        cv2.HOUGH_GRADIENT,
        1,
        40,
        param1=250,
        param2=150,
        minRadius=0,
        maxRadius=0,
    )
    if circles is not None:
        # Convert the (x, y) coordinates and radius of the circles to integers
        circles = np.round(circles[0, :]).astype("int")
    else:
        print("No circles detected")
    for x, y, radius in circles:
        diameter = 2 * radius
    factor = 25 / diameter
    cv2.circle(img, (x, y), radius, (0, 255, 0), 4)
    q.put(resize_img(img, to_rgb=True))
    q.put(None)

    _, binary_img = cv2.threshold(gray, 127, 255, cv2.THRESH_BINARY)
    q.put(resize_img(binary_img))
    q.put(None)

    # edges = cv2.Canny(binary_img, 50, 150)
    # q.put(resize_img(edges))
    # q.put(None)

    # kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    # dilated = cv2.dilate(edges, kernel, iterations=2)
    # q.put(resize_img(dilated))
    # q.put(None)

    # contours, _ = cv2.findContours(dilated, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    # longest_contour = max(contours, key=cv2.contourArea)
    # cv2.drawContours(img, longest_contour, -1, (0, 255, 0), 3)
    # q.put(resize_img(img, to_rgb=True))
    # q.put(None)

    resized = cv2.resize(binary_img, dsize=(0, 0), fx=0.25, fy=0.25)
    opened = cv2.morphologyEx(
        resized, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (7, 9))
    )
    q.put(Image.fromarray(opened))
    q.put(None)

    restored = cv2.resize(opened, dsize=(img.shape[1], img.shape[0]))
    contours, _ = cv2.findContours(restored, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    longest_contour = max(contours, key=cv2.contourArea)
    cv2.drawContours(img, longest_contour, -1, (0, 255, 0), 3)
    q.put(resize_img(img, to_rgb=True))
    q.put(None)

    rect = cv2.minAreaRect(longest_contour)
    (center_x, center_y), (width, height), angle = rect

    axes = (int(width / 2), int(height / 2))

    if width < height:
        angle += 90
    angle = abs(angle)
    if angle > 90:
        angle = 180 - angle

    center = (int(center_x), int(center_y))
    cv2.ellipse(img, center, axes, angle, 0, 360, (0, 255, 0), 2)

    long_axis_direction = np.array([width, 0]).reshape((2, 1)) / np.linalg.norm(width)
    valid_points = []
    for point in longest_contour:
        x, y = point[0]
        local_point = np.array([[x - center_x], [y - center_y]])
        # Here: Minimal rect's height => ellipse's short axis
        rotation_matrix = np.array(
            [
                [math.cos(np.radians(angle)), -math.sin(np.radians(angle))],
                [math.sin(np.radians(angle)), math.cos(np.radians(angle))],
            ]
        )
        local_point_rotated = np.dot(rotation_matrix, local_point)

        dot_product = np.sum(local_point_rotated * long_axis_direction)
        angle_with_long_axis = np.arccos(
            dot_product
            / (
                np.linalg.norm(local_point_rotated)
                * np.linalg.norm(long_axis_direction)
            )
        )

        if angle_with_long_axis <= np.pi / 2:
            valid_points.append(point[0])

    distances = (
        np.sort(
            [
                distance_to_major_axis(point, (center_x, center_y), angle)
                for point in valid_points
            ]
        )
        * factor
    )
    integrals = np.array(
        [
            quad(calc_area, distances[i], distances[i + 1])[0]
            for i in range(len(distances) - 1)
        ]
    )

    total_integral = np.sum(integrals)

    box = cv2.boxPoints(rect)
    box = np.int_(box)

    cv2.drawContours(img, [box], 0, (0, 255, 0), 2)
    q.put(resize_img(img, to_rgb=True))

    return width * factor, height * factor, total_integral


def handle_upload(e):
    global input

    ui.notify(f"Uploaded {e.name}")
    input = cv2.imdecode(np.frombuffer(e.content.read(), np.uint8), cv2.IMREAD_COLOR)


rows = [
    {"parameter": "Major length (mm)", "value": None},
    {"parameter": "Minor length (mm)", "value": None},
    {"parameter": "Volume (mm^3)", "value": None},
]


async def handle_compute():
    width, height, volume = await run.cpu_bound(compute, input, queue)

    rows[0]["value"] = width
    rows[1]["value"] = height
    rows[2]["value"] = volume

    ui.table(
        columns=[
            {
                "name": "parameter",
                "label": "Parameter",
                "field": "parameter",
                "align": "left",
            },
            {"name": "value", "label": "Value", "field": "value"},
        ],
        rows=rows,
        row_key="parameter",
    )


def clear_all():
    """To reset the page.

    **Note**: `ui.navigate.reload()` must be placed at the top.
    """
    ui.navigate.reload()
    stepper.set_value("Gray")
    [i.delete() for i in stepper_imgs]


with ui.left_drawer(top_corner=True, bottom_corner=True):
    ui.label("Please pick the pineapple image:")
    ui.upload(on_upload=handle_upload).classes("max-w-full")

    details_switch = ui.switch("Show the details", value=True)

    ui.button("Compute", on_click=handle_compute)
    ui.button("Reset", on_click=clear_all)

with ui.stepper().props("vertical header-nav").bind_visibility_from(
    details_switch, "value"
) as stepper:
    with ui.step("Gray"):
        ui.label("Transform the image to gray")
    with ui.step("Smoothing"):
        ui.label("Smooth the image")
    with ui.step("Scaling"):
        ui.label("Find the scaler")
    with ui.step("Binary"):
        ui.label("Transform to binary")
    with ui.step("Opening"):
        ui.label("Morphological opening")
    with ui.step("Contour"):
        ui.label("Find the longest contour")
    with ui.step("Fitting"):
        ui.label(
            "Fit minimal rectangle and its inscribed ellipse on the longest contour"
        )

with ui.dialog().props("full-width") as dialog:
    with ui.card():
        zoomed_img = ui.image().props("fit=scale-down")

queue = Manager().Queue(1)

ui.timer(1, callback=lambda: render_steppers(queue) if not queue.empty() else None)

with ui.footer():
    ui.label("CJ © 2024")

ui.run(title="Smart Pineapple")
