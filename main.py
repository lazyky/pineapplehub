#! /usr/bin/env python

from multiprocessing import Manager, Queue
from pineapplehub.calc import (
    distance_to_major_axis,
    calc_volume,
    connect_contours,
    detect_circle,
    remove_hypotenuse,
    get_new_width,
    unwrap,
    close_then_open,
)
import cv2
import numpy as np
from nicegui import run, ui
from PIL import Image, ExifTags
import math
from contextlib import contextmanager
from io import BytesIO
import time
import copy
from pineapplehub.exif import ImageWithExif
from pineapplehub.ui import TABLE_COLUMNS, reset_table
from pineapplehub.result import Result

ui.add_body_html(
    """
    <script>
    const observer = new ResizeObserver(entries => {
        entries.forEach(entry => {
            emitEvent('resize', {
                width: entry.contentRect.width,
                height: entry.contentRect.height,
            });
        });
    });
    document.addEventListener('DOMContentLoaded', () => {
        requestAnimationFrame(async () => {
            const mainElement = document.querySelector('main');
            if (mainElement) {
                // To confirm events will be sent after `ui.on` is ready
                await new Promise(r => setTimeout(r, 5000));
                observer.observe(mainElement);
            }
        });
    });

    window.addEventListener('resize', () => {
        const mainElement = document.querySelector('main');
        if (mainElement) {
            observer.observe(mainElement);
        }
    });
    </script>
"""
)


def resize(e):
    global screen_w, screen_h

    screen_w = e.args.get("width")
    screen_h = e.args.get("height")


ui.on("resize", lambda e: resize(e), throttle=0.1)


def zoom_in(img):
    zoomed_img.set_source(img)
    zoom_dialog.open()


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
    elif isinstance(ctx, str):
        ui.notify(ctx, close_button="GOT", type="negative")
    else:
        with ui.teleport(f"#c{stepper.id} > div:nth-child(1) .q-stepper__step-inner"):
            # Cannot use `ui.interactive_image()` here
            stepper_imgs.append(
                ui.image(ctx).on("click", lambda: zoom_in(ctx)).classes("w-64")
            )


def resize_img(arr, to_rgb=False) -> Image:
    """Resize image:
    The longer size of the image will be equal to the shorter size of the screen.

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
        new_w = int(screen_shorter)
        new_h = int((new_w / w) * h)
    else:
        new_h = int(screen_shorter)
        new_w = int((new_h / h) * w)

    return img.resize((new_w, new_h), Image.Resampling.LANCZOS)


def compute(inputs: ImageWithExif, q):
    # h,  w = inputs.img.shape[:2]
    # newcameramtx, roi = cv2.getOptimalNewCameraMatrix(mtx, dist, (w,h), 1, (w,h))
    # inputs.img = cv2.undistort(inputs.img, mtx, dist, None, newcameramtx)
    # x, y, w, h = roi
    # inputs.img = inputs.img[y:y+h, x:x+w]

    # inputs.img = cv2.rotate(inputs.img, cv2.ROTATE_90_CLOCKWISE)
    raw = copy.deepcopy(inputs.img)

    gray = cv2.cvtColor(inputs.img, cv2.COLOR_BGR2GRAY)
    q.put(resize_img(gray))
    q.put(None)

    smoothed = cv2.GaussianBlur(gray, (7, 7), 0)
    q.put(resize_img(smoothed))
    q.put(None)

    _, binary_img = cv2.threshold(
        smoothed, 127, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU
    )
    q.put(resize_img(binary_img))
    q.put(None)

    resized = cv2.resize(binary_img, dsize=(0, 0), fx=0.25, fy=0.25)
    closed = cv2.morphologyEx(
        resized, cv2.MORPH_CLOSE, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))
    )
    q.put(Image.fromarray(closed))
    q.put(None)

    opened = cv2.morphologyEx(
        closed, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (7, 7))
    )
    q.put(Image.fromarray(opened))
    q.put(None)

    restored = cv2.resize(opened, dsize=(inputs.img.shape[1], inputs.img.shape[0]))

    contours, _ = cv2.findContours(restored, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)

    circles = detect_circle(connect_contours(contours))

    if len(circles) == 0:
        q.put("No scaler (coin) detected! Please check the image or submit a new issue")
        return
    elif len(circles) > 1:
        q.put(
            "Multiple scalers (maybe not coin) detected! Please check the image or submit a new issue"
        )
        return

    contour, diameter = circles[0]
    (x, y), radius = cv2.minEnclosingCircle(contour)
    center = (int(x), int(y))
    radius = int(radius)
    cv2.circle(inputs.img, center, radius, (0, 255, 0), 5)

    q.put(resize_img(inputs.img, True))
    q.put(None)

    contours, _ = cv2.findContours(restored, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    longest_contour = max(remove_hypotenuse(contours), key=cv2.contourArea)
    cv2.drawContours(inputs.img, longest_contour, -1, (0, 255, 0), 3)
    q.put(resize_img(inputs.img, to_rgb=True))
    q.put(None)

    rect = cv2.minAreaRect(longest_contour)
    (center_x, center_y), (width, height), angle = rect

    axes = (int(max(width, height) / 2), int(min(width, height) / 2))

    raw_factor = 25 / diameter

    corrected_factor = 25 / get_new_width(
        inputs.focal_length,
        25,
        inputs.pixel_x_dimension,
        diameter,
        inputs.get_sensor_width_mm(),
        axes[1],
    )

    if width < height:
        angle -= 90
    angle = abs(angle)
    if angle > 90:
        angle = 180 - angle

    center = (int(center_x), int(center_y))

    box = cv2.boxPoints(rect)
    box = np.int_(box)

    width = int(rect[1][0])
    height = int(rect[1][1])

    dst_pts = np.array(
        [[0, height - 1], [0, 0], [width - 1, 0], [width - 1, height - 1]],
        dtype="float32",
    )

    M = cv2.getPerspectiveTransform(box.astype("float32"), dst_pts)
    warped = cv2.warpPerspective(raw, M, (width, height))

    gray = cv2.cvtColor(warped, cv2.COLOR_BGR2GRAY)
    smoothed = cv2.GaussianBlur(gray, (7, 7), 0)
    unwrapped = unwrap(smoothed)

    _, binary_img = cv2.threshold(
        unwrapped, 127, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU
    )

    contours, _ = cv2.findContours(
        close_then_open(binary_img), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE
    )
    longest_contour = max(contours, key=cv2.contourArea)
    rect = cv2.minAreaRect(longest_contour)
    box = cv2.boxPoints(rect)
    box = np.int_(box)
    box_sorted = sorted(box, key=lambda x: x[1])
    top_edge = [box_sorted[0], box_sorted[1]]
    bottom_edge = [box_sorted[2], box_sorted[3]]

    cv2.line(unwrapped, tuple(top_edge[0]), tuple(top_edge[1]), 255, 2)
    cv2.line(unwrapped, tuple(bottom_edge[0]), tuple(bottom_edge[1]), 255, 2)

    top_mid = (
        (top_edge[0][0] + top_edge[1][0]) // 2,
        (top_edge[0][1] + top_edge[1][1]) // 2,
    )
    bottom_mid = (
        (bottom_edge[0][0] + bottom_edge[1][0]) // 2,
        (bottom_edge[0][1] + bottom_edge[1][1]) // 2,
    )

    dash_length = 10
    gap_length = 5
    line_thickness = 2

    dx = bottom_mid[0] - top_mid[0]
    dy = bottom_mid[1] - top_mid[1]
    line_length = np.sqrt(dx**2 + dy**2)

    unit_dx = dx / line_length
    unit_dy = dy / line_length

    current_length = 0
    while current_length < line_length:
        start = (
            int(top_mid[0] + unit_dx * current_length),
            int(top_mid[1] + unit_dy * current_length),
        )
        end = (
            int(top_mid[0] + unit_dx * (current_length + dash_length)),
            int(top_mid[1] + unit_dy * (current_length + dash_length)),
        )

        cv2.line(unwrapped, start, end, 255, line_thickness)

        current_length += dash_length + gap_length

    q.put(resize_img(cv2.hconcat([smoothed, unwrapped])))

    # Vertical, with correct height
    if warped.shape[0] >= warped.shape[1]:
        final_height = max(rect[1])
    # Horizonal, with correct distances
    else:
        true_rect = copy.deepcopy(rect)
        final_width = min(rect[1])

    rotated = cv2.rotate(smoothed, cv2.ROTATE_90_CLOCKWISE)
    unwrapped = unwrap(rotated)
    _, binary_img = cv2.threshold(
        unwrapped, 127, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU
    )
    contours, _ = cv2.findContours(
        close_then_open(binary_img), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE
    )
    longest_contour = max(contours, key=cv2.contourArea)
    rect = cv2.minAreaRect(longest_contour)
    box = cv2.boxPoints(rect)
    box = np.int_(box)
    box_sorted = sorted(box, key=lambda x: x[1])  # Sort by y
    top_edge = [box_sorted[0], box_sorted[1]]
    bottom_edge = [box_sorted[2], box_sorted[3]]

    cv2.line(unwrapped, tuple(top_edge[0]), tuple(top_edge[1]), 255, 2)
    cv2.line(unwrapped, tuple(bottom_edge[0]), tuple(bottom_edge[1]), 255, 2)

    top_mid = (
        (top_edge[0][0] + top_edge[1][0]) // 2,
        (top_edge[0][1] + top_edge[1][1]) // 2,
    )
    bottom_mid = (
        (bottom_edge[0][0] + bottom_edge[1][0]) // 2,
        (bottom_edge[0][1] + bottom_edge[1][1]) // 2,
    )

    dash_length = 10
    gap_length = 5
    line_thickness = 2

    # Slope and length
    dx = bottom_mid[0] - top_mid[0]
    dy = bottom_mid[1] - top_mid[1]
    line_length = np.sqrt(dx**2 + dy**2)

    # Direction vector
    unit_dx = dx / line_length
    unit_dy = dy / line_length

    # Draw dash line
    current_length = 0
    while current_length < line_length:
        start = (
            int(top_mid[0] + unit_dx * current_length),
            int(top_mid[1] + unit_dy * current_length),
        )
        end = (
            int(top_mid[0] + unit_dx * (current_length + dash_length)),
            int(top_mid[1] + unit_dy * (current_length + dash_length)),
        )

        cv2.line(unwrapped, start, end, 255, line_thickness)

        current_length += dash_length + gap_length

    # Vertical, with correct height
    if warped.shape[0] >= warped.shape[1]:
        true_rect = copy.deepcopy(rect)
        final_width = min(rect[1])
    # Horizonal, with correct distances
    else:
        final_height = max(rect[1])
        

    (center_x, center_y), (width, height), angle = true_rect
    long_axis_direction = np.array([width, 0]).reshape((2, 1)) / np.linalg.norm(
        width
    )
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

    distances_pixels = np.sort(
        [
            distance_to_major_axis(point, (center_x, center_y), angle)
            for point in valid_points
        ]
    )

    raw_distances = distances_pixels * raw_factor
    corrected_distances = distances_pixels * corrected_factor

    q.put(resize_img(cv2.hconcat([rotated, unwrapped])))


    return (
        Result(
            final_width * raw_factor,
            final_height * raw_factor,
            calc_volume(raw_distances),
        ),
        Result(
            final_width * corrected_factor,
            final_height * corrected_factor,
            calc_volume(corrected_distances),
        ),
    )


async def handle_upload(e):
    if raw_rows[0]["value"] is not None:
        choose = await confirm_dialog
        if choose == "Yes":
            clear_all()
        else:
            ui.notify("User canceled.")
            return

    global inputs

    buffer = e.content.read()

    exif = Image.open(BytesIO(buffer)).getexif()
    exif_ifd = exif.get_ifd(ExifTags.IFD.Exif)

    try:
        inputs = ImageWithExif(
            cv2.imdecode(np.frombuffer(buffer, np.uint8), cv2.IMREAD_COLOR),
            exif_ifd[ExifTags.Base.FocalLength],
            exif_ifd[ExifTags.Base.ExifImageWidth],
            exif_ifd[ExifTags.Base.FocalPlaneXResolution],
        )
    except KeyError:
        ui.notify(
            "Missing EXIF! Do NOT edit the photo by yourself.",
            close_button="GOT",
            type="negative",
        )
        e.sender.reset()
    else:
        ui.notify(f"Uploaded {e.name}")


raw_rows = [
    {"parameter": "Major length (mm)", "value": None},
    {"parameter": "Minor length (mm)", "value": None},
    {"parameter": "Volume (mm^3)", "value": None},
]

corrected_rows = copy.deepcopy(raw_rows)


@contextmanager
def disable(button: ui.button):
    button.disable()
    try:
        yield
    finally:
        button.enable()


async def handle_compute(button: ui.button):
    try:
        inputs
    except NameError:
        ui.notify("The image file must be uploaded", type="negative")
        return
    else:
        origin_detail_status = True
        if not details_switch.value:
            origin_detail_status = False
            details_switch.set_value(True)

        with disable(button):
            reset_button.disable()
            try:
                raw_results, corrected_results = await run.cpu_bound(
                    compute, inputs, queue
                )
            except Exception as e:
                ui.notify(e, close_button="GOT", type="negative")
            else:
                raw_rows[0]["value"] = raw_results.major
                raw_rows[1]["value"] = raw_results.minor
                raw_rows[2]["value"] = raw_results.volume

                corrected_rows[0]["value"] = corrected_results.major
                corrected_rows[1]["value"] = corrected_results.minor
                corrected_rows[2]["value"] = corrected_results.volume
                table_raw.update()
                table_corrected.update()

            finally:
                # To wait job in timer finished
                time.sleep(1)
                details_switch.set_value(origin_detail_status)
                reset_button.enable()


def clear_all():
    """To reset the page.

    **Note**: `ui.navigate.reload()` must be placed at the top.
    """
    ui.navigate.reload()
    stepper.set_value("Gray")
    try:
        # Should call `.delete()` here
        [i.delete() for i in stepper_imgs]
        # `.delete()` will not remove the element from the list,
        # but the remaining ones do NOT have `.delete()` method anymore
        stepper_imgs.clear()
        reset_table(raw_rows)
        reset_table(corrected_rows)
    except (ValueError, NameError) as e:
        ui.notify(e, close_button="GOT", type="negative")
    finally:
        uploader.clear()


def handle_markers(e):
    global mtx, dist

    all_object_points = []
    all_image_points = []

    for marker in e.contents:
        gray = cv2.imdecode(
            np.frombuffer(marker.read(), np.uint8), cv2.IMREAD_GRAYSCALE
        )
        board = cv2.aruco.CharucoBoard(
            (5, 7),
            0.03,
            0.015,
            cv2.aruco.getPredefinedDictionary(cv2.aruco.DICT_5X5_100),
        )
        detector = cv2.aruco.CharucoDetector(board)
        charuco_corners, charuco_ids, marker_corners, marker_ids = detector.detectBoard(
            gray
        )

        if len(charuco_ids) >= 4 and len(marker_corners) > 3:
            object_points, image_points = board.matchImagePoints(
                charuco_corners, charuco_ids
            )
            all_object_points.append(object_points)
            all_image_points.append(image_points)

    ret, mtx, dist, rvecs, tvecs = cv2.calibrateCamera(
        all_object_points, all_image_points, gray.shape[::-1], None, None
    )


with ui.dialog() as calib_dialog, ui.card():
    ui.upload(
        multiple=True,
        on_upload=lambda e: ui.notify(f"Uploaded {e.name}"),
        on_multi_upload=handle_markers,
    ).classes("max-w-full")

with ui.left_drawer(top_corner=True, bottom_corner=True):
    ui.label("Please pick the pineapple image:")
    uploader = ui.upload(on_upload=handle_upload).classes("max-w-full")

    details_switch = ui.switch("Show the details", value=True)

    ui.button("Calibrate", on_click=calib_dialog.open)
    ui.button("Compute", on_click=lambda e: handle_compute(e.sender))
    reset_button = ui.button("Reset", on_click=clear_all)

with ui.row():
    with (
        ui.stepper()
        .props("vertical header-nav")
        .bind_visibility_from(details_switch, "value") as stepper
    ):
        with ui.step("Gray"):
            ui.label("Transform the image to gray")
        with ui.step("Smoothing"):
            ui.label("Smooth the image")
        with ui.step("Binary"):
            ui.label("Transform to binary")
        with ui.step("Closing"):
            ui.label("Morphological closing")
        with ui.step("Opening"):
            ui.label("Morphological opening")
        with ui.step("Scaling"):
            ui.label("Find the scaler")
        with ui.step("Contour"):
            ui.label("Find the longest contour")
        with ui.step("Unwrapping"):
            ui.markdown("Warp perspective then unwrap")

    with ui.column():
        table_raw = ui.table(
            title="Raw Results",
            columns=TABLE_COLUMNS,
            rows=raw_rows,
            row_key="parameter",
        )

        table_corrected = ui.table(
            title="Corrected Results",
            columns=TABLE_COLUMNS,
            rows=corrected_rows,
            row_key="parameter",
        )

    with open("doc.md", "r") as f:
        doc = f.read()
    ui.markdown(doc)

with (
    ui.header(elevated=True)
    .style("background-color: #3874c8")
    .classes("items-center justify-between")
):
    ui.label("Pineapple Hub")
    ui.space()
    ui.button(
        "Change Log", on_click=lambda: right_drawer.toggle(), icon="timeline"
    ).props("flat color=white")
    ui.button(
        "BUG REPORT",
        icon="bug_report",
        on_click=lambda: ui.navigate.to(
            "https://git.bigdick.live/ysun/pineapplehub/issues/new"
        ),
    ).props("flat color=white")

with (
    ui.right_drawer(fixed=False)
    .style("background-color: #ebf1fa")
    .props("bordered") as right_drawer
):
    with open("CHANGELOG.md", "r") as f:
        changelog = f.read()
    ui.markdown(changelog)

with ui.dialog().props("full-width") as zoom_dialog:
    with ui.card():
        zoomed_img = ui.image().props("fit=scale-down")

with ui.dialog() as confirm_dialog, ui.card():
    ui.markdown(
        """
        **Previous** results detected.

        All results will **be cleared** before the next calculation.

        Please make sure you have all needed data **marked down**.
        """
    )
    with ui.row():
        ui.button("Yes", on_click=lambda: confirm_dialog.submit("Yes"))
        ui.button("No", on_click=lambda: confirm_dialog.submit("No"))


queue = Manager().Queue(1)

ui.timer(1, callback=lambda: render_steppers(queue) if not queue.empty() else None)

with ui.footer():
    ui.label("CJ © 2024")

ui.run(title="PineappleHub", favicon="🍍")
