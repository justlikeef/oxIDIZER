from PIL import Image
import sys

def crop_image(image_path):
    try:
        img = Image.open(image_path)
        img = img.convert("RGBA")
        
        # GetBoundingBox returns the box of non-zero regions
        # If alpha is used for transparency, this usually works on the alpha channel.
        # However, getbbox() works on the "union of all bands".
        # Better to check alpha specifically if possible, but getbbox is standard.
        
        bbox = img.getbbox()
        if bbox:
            cropped = img.crop(bbox)
            cropped.save(image_path)
            print(f"Successfully cropped {image_path} to {bbox}")
        else:
            print("No content found to crop (image might be empty or fully transparent).")

    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 crop_logo.py <image_path>")
        sys.exit(1)
    
    crop_image(sys.argv[1])
