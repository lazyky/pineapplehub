#include <opencv2/core/core.hpp>
#include <opencv2/highgui/highgui.hpp>
#include <opencv2/imgproc/imgproc.hpp>
#include <iostream>

cv::Point2f convert_pt(cv::Point2f point, int w, int h)
{
    cv::Point2f pc(point.x - w / 2, point.y - h / 2);

    float f = w;
    float r = w;

    float omega = w / 2;
    float z0 = f - sqrt(r * r - omega * omega);

    float zc = (2 * z0 + sqrt(4 * z0 * z0 - 4 * (pc.x * pc.x / (f * f) + 1) * (z0 * z0 - r * r))) / (2 * (pc.x * pc.x / (f * f) + 1));
    cv::Point2f final_point(pc.x * zc / f, pc.y * zc / f);
    final_point.x += w / 2;
    final_point.y += h / 2;
    return final_point;
}

void resizeImage(const cv::Mat &image, cv::Mat &dest_im, int width, int height)
{
    dest_im.create(height, width, image.type());

    for (int y = 0; y < height; ++y)
    {
        for (int x = 0; x < width; ++x)
        {
            cv::Point2f current_pos(x, y);
            current_pos = convert_pt(current_pos, width, height);

            cv::Point2i top_left((int)current_pos.x,
                                 (int)current_pos.y);

            if (top_left.x < 0 || top_left.x > width - 2 ||
                top_left.y < 0 || top_left.y > height - 2)
            {
                continue;
            }

            float dx = current_pos.x - top_left.x;
            float dy = current_pos.y - top_left.y;

            float weight_tl = (1.0 - dx) * (1.0 - dy);
            float weight_tr = dx * (1.0 - dy);
            float weight_bl = (1.0 - dx) * dy;
            float weight_br = dx * dy;

            uchar value = weight_tl * image.at<uchar>(top_left.y * image.cols + top_left.x) +
                          weight_tr * image.at<uchar>((top_left.y + 1) * image.cols + top_left.x) +
                          weight_bl * image.at<uchar>(top_left.y * image.cols + (top_left.x + 1)) +
                          weight_br * image.at<uchar>((top_left.y + 1) * image.cols + (top_left.x + 1));

            dest_im.at<uchar>(y, x) = value;
        }
    }
}

int main()
{
    cv::Mat image = cv::imread("3.jpg", cv::IMREAD_GRAYSCALE);
    if (image.empty())
    {
        std::cerr << "Could not open or read the image file." << std::endl;
        return -1;
    }

    int width = image.cols;
    std::cout << width << std::endl;
    int height = image.rows;

    cv::Mat dest_image;
    resizeImage(image, dest_image, width, height);

    cv::imwrite("output_resized_image.jpg", dest_image);

    return 0;
}