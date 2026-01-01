#!/usr/bin/env python3
"""
Convert arceos.png logo to raw pixel data (u32 ARGB format) for embedding in Rust.
Output: 1920x1200 image with white background, logo centered.
"""

from PIL import Image
import os
import struct

def convert_logo_to_raw(input_path: str, output_path: str, width: int = 1920, height: int = 1200):
    """
    Convert logo to raw pixel data (ARGB32 format, little-endian).
    
    Args:
        input_path: Path to the input logo image
        output_path: Path to save the raw pixel data
        width: Target width (default: 1920)
        height: Target height (default: 1200)
    """
    # Create white background (RGBA)
    background = Image.new('RGBA', (width, height), (255, 255, 255, 255))
    
    # Open the logo
    logo = Image.open(input_path)
    
    # Convert to RGBA if not already (to handle transparency)
    if logo.mode != 'RGBA':
        logo = logo.convert('RGBA')
    
    # Calculate position to center the logo
    logo_width, logo_height = logo.size
    
    # If logo is larger than the canvas, scale it down while maintaining aspect ratio
    if logo_width > width or logo_height > height:
        # Calculate scale factor (80% of available space)
        scale = min(width / logo_width, height / logo_height) * 0.8
        new_width = int(logo_width * scale)
        new_height = int(logo_height * scale)
        logo = logo.resize((new_width, new_height), Image.Resampling.LANCZOS)
        logo_width, logo_height = logo.size
    
    # Calculate centered position
    x = (width - logo_width) // 2
    y = (height - logo_height) // 2
    
    # Paste the logo onto the white background
    background.paste(logo, (x, y), logo)
    
    # Convert to RGB (framebuffer doesn't need alpha)
    background = background.convert('RGB')
    
    # Write raw pixel data (each pixel as u32: 0x00RRGGBB)
    with open(output_path, 'wb') as f:
        pixels = background.load()
        for row in range(height):
            for col in range(width):
                r, g, b = pixels[col, row]
                # Pack as little-endian u32: 0x00RRGGBB
                pixel_value = (r << 16) | (g << 8) | b
                f.write(struct.pack('<I', pixel_value))
    
    # Calculate file size
    file_size = width * height * 4
    print(f"Saved: {output_path}")
    print(f"  Dimensions: {width}x{height}")
    print(f"  File size: {file_size} bytes ({file_size / 1024 / 1024:.2f} MB)")
    print(f"  Logo size: {logo_width}x{logo_height}")
    print(f"  Logo position: ({x}, {y})")

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    input_path = os.path.join(script_dir, "arceos.png")
    output_path = os.path.join(script_dir, "logo.raw")
    
    if not os.path.exists(input_path):
        print(f"Error: Input file not found: {input_path}")
        return 1
    
    convert_logo_to_raw(input_path, output_path)
    return 0

if __name__ == "__main__":
    exit(main())
