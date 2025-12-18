from PIL import Image
import sys

def get_dominant_color(image_path):
    try:
        img = Image.open(image_path)
        img = img.resize((150, 150))
        img = img.convert("RGBA")
        
        # Get colors, excluding fully transparent
        colors = img.getcolors(maxcolors=22500)
        # Sort by count
        colors.sort(key=lambda x: x[0], reverse=True)
        
        valid_colors = []
        for count, color in colors:
            # color is (r, g, b, a)
            if color[3] < 128: continue # Ignore transparent
            if color[0] < 20 and color[1] < 20 and color[2] < 20: continue # Ignore near-black
            
            hex_color = "#{:02x}{:02x}{:02x}".format(color[0], color[1], color[2])
            valid_colors.append(hex_color)
            if len(valid_colors) >= 3: break
            
        if not valid_colors:
             return "#3b82f6" # Default if only black/transparent found
             
        return valid_colors[0]
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        return "#3b82f6" # Default blue

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 extract_color.py <image_path>")
        sys.exit(1)
    
    print(get_dominant_color(sys.argv[1]))
