#!/usr/bin/env python3
"""Generate macOS App Store screenshots for eudamed2firstbase."""

from PIL import Image, ImageDraw, ImageFont
import os

W, H = 2560, 1600
BG = (30, 30, 30)           # dark background (egui dark mode)
PANEL = (39, 39, 39)        # panel bg
WIDGET_BG = (50, 50, 50)    # input field bg
BORDER = (70, 70, 70)       # borders
TEXT = (220, 220, 220)       # primary text
TEXT_DIM = (150, 150, 150)   # dimmed text
ACCENT = (100, 160, 255)    # blue accent
GREEN = (80, 200, 120)      # success
BUTTON_BG = (60, 100, 180)  # button
BUTTON_TEXT = (255, 255, 255)
SEPARATOR = (60, 60, 60)
RADIO_ON = ACCENT
RADIO_OFF = (80, 80, 80)
CHECKBOX_ON = ACCENT
TITLE_BAR = (44, 44, 44)    # macOS dark title bar
TRAFFIC_RED = (255, 95, 86)
TRAFFIC_YELLOW = (255, 189, 46)
TRAFFIC_GREEN = (39, 201, 63)

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ICON_PATH = os.path.join(SCRIPT_DIR, "assets", "icon_256x256.png")
OUT_DIR = os.path.join(SCRIPT_DIR, "screenshots")

# Try to load system fonts
def get_fonts():
    paths = [
        "/System/Library/Fonts/SFNSMono.ttf",
        "/System/Library/Fonts/Supplemental/SF-Mono-Regular.otf",
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Supplemental/Menlo.ttc",
        "/Library/Fonts/SF-Mono-Regular.otf",
    ]
    mono = None
    for p in paths:
        if os.path.exists(p):
            try:
                mono = ImageFont.truetype(p, 28)
                break
            except:
                pass

    sans_paths = [
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/SFNSText.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/HelveticaNeue.ttc",
        "/Library/Fonts/Arial.ttf",
    ]
    sans = None
    for p in sans_paths:
        if os.path.exists(p):
            try:
                sans = ImageFont.truetype(p, 30)
                break
            except:
                pass

    sans_bold = None
    bold_paths = [
        "/System/Library/Fonts/SFNSTextBold.ttf",
        "/System/Library/Fonts/Supplemental/Helvetica Bold.ttc",
    ]
    for p in bold_paths:
        if os.path.exists(p):
            try:
                sans_bold = ImageFont.truetype(p, 38)
                break
            except:
                pass

    if mono is None:
        mono = ImageFont.load_default()
    if sans is None:
        sans = mono
    if sans_bold is None:
        sans_bold = sans
    return sans, sans_bold, mono

SANS, SANS_BOLD, MONO = get_fonts()

# Create larger font variants
def sized_font(base_path_list, size):
    for p in base_path_list:
        if os.path.exists(p):
            try:
                return ImageFont.truetype(p, size)
            except:
                pass
    return SANS

SANS_SMALL = sized_font(["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"], 26)
SANS_LABEL = sized_font(["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"], 30)
SANS_HEADING = sized_font(["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"], 42)
MONO_LOG = sized_font(["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc", "/System/Library/Fonts/Supplemental/Menlo.ttc"], 24)

def draw_macos_titlebar(draw, title="eudamed2firstbase"):
    """Draw macOS-style dark title bar with traffic lights."""
    # Title bar background
    draw.rectangle([0, 0, W, 52], fill=TITLE_BAR)
    draw.line([0, 52, W, 52], fill=BORDER, width=1)
    # Traffic lights
    cx, cy = 30, 26
    for i, color in enumerate([TRAFFIC_RED, TRAFFIC_YELLOW, TRAFFIC_GREEN]):
        x = cx + i * 40
        draw.ellipse([x-10, cy-10, x+10, cy+10], fill=color)
    # Title centered
    bbox = draw.textbbox((0,0), title, font=SANS_SMALL)
    tw = bbox[2] - bbox[0]
    draw.text(((W - tw)//2, 14), title, fill=TEXT_DIM, font=SANS_SMALL)

def draw_rounded_rect(draw, xy, radius, fill=None, outline=None):
    x0, y0, x1, y1 = xy
    if fill:
        draw.rounded_rectangle(xy, radius=radius, fill=fill, outline=outline)

def draw_text_input(draw, x, y, w, h, text="", hint="", font=None):
    """Draw a text input field."""
    if font is None:
        font = SANS_LABEL
    draw_rounded_rect(draw, [x, y, x+w, y+h], radius=6, fill=WIDGET_BG, outline=BORDER)
    if text:
        draw.text((x+12, y+8), text, fill=TEXT, font=font)
    elif hint:
        draw.text((x+12, y+8), hint, fill=TEXT_DIM, font=font)

def draw_multiline_input(draw, x, y, w, h, lines=None, hint_lines=None, font=None):
    """Draw a multiline text input."""
    if font is None:
        font = SANS_LABEL
    draw_rounded_rect(draw, [x, y, x+w, y+h], radius=6, fill=WIDGET_BG, outline=BORDER)
    if lines:
        for i, line in enumerate(lines):
            draw.text((x+12, y+10+i*36), line, fill=TEXT, font=font)
    elif hint_lines:
        for i, line in enumerate(hint_lines):
            draw.text((x+12, y+10+i*36), line, fill=TEXT_DIM, font=font)

def draw_button(draw, x, y, w, h, text, enabled=True, accent=False):
    """Draw a button."""
    bg = BUTTON_BG if (enabled and accent) else BUTTON_BG if enabled else (50, 50, 50)
    fg = BUTTON_TEXT if enabled else TEXT_DIM
    draw_rounded_rect(draw, [x, y, x+w, y+h], radius=8, fill=bg, outline=BORDER)
    bbox = draw.textbbox((0,0), text, font=SANS_LABEL)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    draw.text((x+(w-tw)//2, y+(h-th)//2 - 2), text, fill=fg, font=SANS_LABEL)

def draw_radio(draw, x, y, selected=False, label=""):
    """Draw a radio button with label."""
    r = 12
    outline_c = RADIO_ON if selected else RADIO_OFF
    draw.ellipse([x, y, x+2*r, y+2*r], outline=outline_c, width=2)
    if selected:
        draw.ellipse([x+4, y+4, x+2*r-4, y+2*r-4], fill=RADIO_ON)
    draw.text((x+2*r+10, y-4), label, fill=TEXT, font=SANS_LABEL)

def draw_checkbox(draw, x, y, checked=False, label=""):
    """Draw a checkbox with label."""
    s = 24
    draw_rounded_rect(draw, [x, y, x+s, y+s], radius=4, fill=CHECKBOX_ON if checked else WIDGET_BG, outline=BORDER)
    if checked:
        # checkmark
        draw.line([x+5, y+12, x+10, y+18], fill=(255,255,255), width=3)
        draw.line([x+10, y+18, x+19, y+6], fill=(255,255,255), width=3)
    draw.text((x+s+10, y-2), label, fill=TEXT, font=SANS_LABEL)

def draw_log_area(draw, x, y, w, h, lines, font=None):
    """Draw a log/console area with text lines."""
    if font is None:
        font = MONO_LOG
    draw_rounded_rect(draw, [x, y, x+w, y+h], radius=6, fill=(25, 25, 25), outline=BORDER)
    clip_y = y + 10
    for line in lines:
        if clip_y + 28 > y + h - 10:
            break
        color = TEXT
        if line.startswith("=== DONE"):
            color = GREEN
        elif line.startswith("=== FAILED"):
            color = (255, 100, 100)
        elif line.startswith("["):
            color = ACCENT
        draw.text((x+14, clip_y), line, fill=color, font=font)
        clip_y += 30

def add_icon(img, x, y, size=80):
    """Overlay the app icon."""
    try:
        icon = Image.open(ICON_PATH).convert("RGBA")
        icon = icon.resize((size, size), Image.LANCZOS)
        img.paste(icon, (x, y), icon)
    except:
        pass


def screenshot_1_main():
    """Screenshot 1: Main window, empty state with hint text."""
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw_macos_titlebar(draw)

    margin = 60
    y = 80

    # Icon + heading
    add_icon(img, W - margin - 80, y + 4, 80)
    draw.text((margin, y), "eudamed2firstbase", fill=TEXT, font=SANS_HEADING)
    y += 70

    # SRN label + input
    draw.text((margin, y), "SRNs (one per line or space-separated):", fill=TEXT, font=SANS_LABEL)
    y += 44
    draw_multiline_input(draw, margin, y, W - 2*margin, 130,
                         hint_lines=["DE-MF-000012345", "FR-MF-000067890"])
    y += 150

    # Options row
    draw.text((margin, y+4), "Limit per SRN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 140, 42, hint="all")
    draw_checkbox(draw, margin+420, y+10, checked=False, label="Dry run (download & convert only)")
    y += 66

    # Target selector
    draw.text((margin, y+2), "Target:", fill=TEXT, font=SANS_LABEL)
    draw_radio(draw, margin+130, y+6, selected=True, label="GS1 firstbase")
    draw_radio(draw, margin+380, y+6, selected=False, label="Swissdamed")
    y += 56

    # Credentials section (collapsed)
    draw.text((margin, y), "▶ GS1 firstbase Credentials", fill=ACCENT, font=SANS_LABEL)
    y += 52

    # Button
    draw_button(draw, margin, y, 500, 52, "Download, Convert & Push to firstbase", enabled=False, accent=True)
    y += 76

    # Separator
    draw.line([margin, y, W-margin, y], fill=SEPARATOR, width=1)
    y += 16

    # Log label
    draw.text((margin, y), "Log:", fill=TEXT, font=SANS_LABEL)
    y += 40

    # Empty log area
    draw_log_area(draw, margin, y, W-2*margin, H-y-40, [])

    img.save(os.path.join(OUT_DIR, "screenshot_1_main.png"))
    print("Saved screenshot_1_main.png")


def screenshot_2_running():
    """Screenshot 2: Download running with log output."""
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw_macos_titlebar(draw)

    margin = 60
    y = 80

    add_icon(img, W - margin - 80, y + 4, 80)
    draw.text((margin, y), "eudamed2firstbase", fill=TEXT, font=SANS_HEADING)
    y += 70

    draw.text((margin, y), "SRNs (one per line or space-separated):", fill=TEXT, font=SANS_LABEL)
    y += 44
    draw_multiline_input(draw, margin, y, W - 2*margin, 130,
                         lines=["DE-MF-000017808"])
    y += 150

    draw.text((margin, y+4), "Limit per SRN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 140, 42, text="50")
    draw_checkbox(draw, margin+420, y+10, checked=False, label="Dry run (download & convert only)")
    y += 66

    draw.text((margin, y+2), "Target:", fill=TEXT, font=SANS_LABEL)
    draw_radio(draw, margin+130, y+6, selected=True, label="GS1 firstbase")
    draw_radio(draw, margin+380, y+6, selected=False, label="Swissdamed")
    y += 56

    draw.text((margin, y), "▶ GS1 firstbase Credentials", fill=ACCENT, font=SANS_LABEL)
    y += 52

    # Button (running)
    draw_button(draw, margin, y, 500, 52, "Running...", enabled=False)
    y += 76

    draw.line([margin, y, W-margin, y], fill=SEPARATOR, width=1)
    y += 16

    draw.text((margin, y), "Log:", fill=TEXT, font=SANS_LABEL)
    y += 40

    log_lines = [
        "[Download] Starting download for SRN: DE-MF-000017808",
        "[Download] Fetching listing page 0 (pageSize=300)...",
        "[Download] Found 247 devices on page 0",
        "[Download] Limiting to 50 devices",
        "[Download] Checking versions... 12 unchanged, 38 need download",
        "[Download] Downloading detail 1/38: 4f1e3733-2987-4d3b-a4fe-aca49455bc0a",
        "[Download] Downloading detail 2/38: 7cd1d81c-b335-4f95-bec0-079f5f4a41a3",
        "[Download] Downloading detail 3/38: a87f1218-0aa5-4427-96cc-9bdaee2a3bbc",
        "[Download] Downloading detail 4/38: 3c298386-e47c-411a-b4d9-123781c6a9ac",
        "[Download] Downloading detail 5/38: 9bd4b6bb-3065-4558-8a93-a5508ce9674b",
        "[Download] Downloading detail 6/38: cb744f68-5ea4-48d3-b4ad-2ad417cfa16b",
        "[Download] Downloading detail 7/38: 6e3662db-ecc9-43d1-8454-38b7064b30e2",
        "[Download] Downloading detail 8/38: e4a1b3c5-9f87-4321-abcd-1234567890ef",
        "[Download] Downloading basic UDI-DI 1/38...",
        "[Download] Downloading basic UDI-DI 2/38...",
        "[Download] Downloading basic UDI-DI 3/38...",
        "[Convert] Converting 38 devices to firstbase JSON...",
        "[Convert] Processing 4f1e3733... → GTIN 04260500560049",
        "[Convert] Processing 7cd1d81c... → GTIN 04260500560056",
    ]
    draw_log_area(draw, margin, y, W-2*margin, H-y-40, log_lines)

    img.save(os.path.join(OUT_DIR, "screenshot_2_running.png"))
    print("Saved screenshot_2_running.png")


def screenshot_3_done():
    """Screenshot 3: Completed pipeline with success summary."""
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw_macos_titlebar(draw)

    margin = 60
    y = 80

    add_icon(img, W - margin - 80, y + 4, 80)
    draw.text((margin, y), "eudamed2firstbase", fill=TEXT, font=SANS_HEADING)
    y += 70

    draw.text((margin, y), "SRNs (one per line or space-separated):", fill=TEXT, font=SANS_LABEL)
    y += 44
    draw_multiline_input(draw, margin, y, W - 2*margin, 130,
                         lines=["DE-MF-000017808"])
    y += 150

    draw.text((margin, y+4), "Limit per SRN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 140, 42, text="50")
    draw_checkbox(draw, margin+420, y+10, checked=False, label="Dry run (download & convert only)")
    y += 66

    draw.text((margin, y+2), "Target:", fill=TEXT, font=SANS_LABEL)
    draw_radio(draw, margin+130, y+6, selected=True, label="GS1 firstbase")
    draw_radio(draw, margin+380, y+6, selected=False, label="Swissdamed")
    y += 56

    draw.text((margin, y), "▶ GS1 firstbase Credentials", fill=ACCENT, font=SANS_LABEL)
    y += 52

    # Button (ready again)
    draw_button(draw, margin, y, 500, 52, "Download, Convert & Push to firstbase", enabled=True, accent=True)
    y += 76

    draw.line([margin, y, W-margin, y], fill=SEPARATOR, width=1)
    y += 16

    draw.text((margin, y), "Log:", fill=TEXT, font=SANS_LABEL)
    y += 40

    log_lines = [
        "[Download] Starting download for SRN: DE-MF-000017808",
        "[Download] Fetching listing page 0 (pageSize=300)...",
        "[Download] Found 247 devices, limiting to 50",
        "[Download] Version check: 12 unchanged, 38 need download",
        "[Download] Downloaded 38/38 details + 38/38 basic UDI-DI",
        "[Convert] Converting 38 devices to firstbase JSON...",
        "[Convert] 38 converted, 0 skipped (unchanged), 0 errors",
        "[Convert] Change summary: 15 NEW, 8 MFR+CERT, 10 MARKET+STATUS, 5 PKG",
        "[Push] Authenticating with GS1 firstbase API...",
        "[Push] CreateMany batch 1/1 (38 items)...",
        "[Push] Polling request ae72f9b5... status: Processing",
        "[Push] Polling request ae72f9b5... status: Done",
        "[Push] AddMany: publishing 38 items + 12 child GTINs to 7612345000527",
        "[Push] Polling request c4239e9c... status: Processing",
        "[Push] Polling request c4239e9c... status: Done",
        "[Push] Results: 38 ACCEPTED, 0 REJECTED",
        "[Push] Moved 38 files to firstbase_json/processed/",
        "",
        "=== DONE === Downloaded 38, converted 38, pushed 38 (0 rejected)",
    ]
    draw_log_area(draw, margin, y, W-2*margin, H-y-40, log_lines)

    img.save(os.path.join(OUT_DIR, "screenshot_3_done.png"))
    print("Saved screenshot_3_done.png")


def screenshot_4_swissdamed():
    """Screenshot 4: Swissdamed target selected with credentials expanded."""
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw_macos_titlebar(draw)

    margin = 60
    y = 80

    add_icon(img, W - margin - 80, y + 4, 80)
    draw.text((margin, y), "eudamed2firstbase", fill=TEXT, font=SANS_HEADING)
    y += 70

    draw.text((margin, y), "SRNs (one per line or space-separated):", fill=TEXT, font=SANS_LABEL)
    y += 44
    draw_multiline_input(draw, margin, y, W - 2*margin, 130,
                         lines=["DE-MF-000017808", "FR-MF-000023456"])
    y += 150

    draw.text((margin, y+4), "Limit per SRN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 140, 42, hint="all")
    draw_checkbox(draw, margin+420, y+10, checked=True, label="Dry run (download & convert only)")
    y += 66

    draw.text((margin, y+2), "Target:", fill=TEXT, font=SANS_LABEL)
    draw_radio(draw, margin+130, y+6, selected=False, label="GS1 firstbase")
    draw_radio(draw, margin+380, y+6, selected=True, label="Swissdamed")
    y += 56

    # Credentials expanded
    draw.text((margin, y), "▼ Swissdamed Credentials", fill=ACCENT, font=SANS_LABEL)
    y += 44

    # Client ID
    draw.text((margin+20, y+6), "Client ID:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+200, y, 500, 42, text="my-client-id-xxxxx")
    y += 56

    # Client Secret
    draw.text((margin+20, y+6), "Client Secret:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+200, y, 500, 42, text="••••••••••••••••")
    y += 56

    # Base URL
    draw.text((margin+20, y+6), "API Base URL:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+200, y, 500, 42, text="https://playground.swissdamed.ch")
    y += 72

    # Button
    draw_button(draw, margin, y, 400, 52, "Download & Convert", enabled=True, accent=True)
    y += 76

    draw.line([margin, y, W-margin, y], fill=SEPARATOR, width=1)
    y += 16

    draw.text((margin, y), "Log:", fill=TEXT, font=SANS_LABEL)
    y += 40

    draw_log_area(draw, margin, y, W-2*margin, H-y-40, [])

    img.save(os.path.join(OUT_DIR, "screenshot_4_swissdamed.png"))
    print("Saved screenshot_4_swissdamed.png")


def screenshot_5_firstbase_creds():
    """Screenshot 5: GS1 firstbase with credentials expanded."""
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)
    draw_macos_titlebar(draw)

    margin = 60
    y = 80

    add_icon(img, W - margin - 80, y + 4, 80)
    draw.text((margin, y), "eudamed2firstbase", fill=TEXT, font=SANS_HEADING)
    y += 70

    draw.text((margin, y), "SRNs (one per line or space-separated):", fill=TEXT, font=SANS_LABEL)
    y += 44
    draw_multiline_input(draw, margin, y, W - 2*margin, 130,
                         lines=["DE-MF-000017808"])
    y += 150

    draw.text((margin, y+4), "Limit per SRN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 140, 42, text="100")
    draw_checkbox(draw, margin+420, y+10, checked=False, label="Dry run (download & convert only)")
    y += 66

    draw.text((margin, y+2), "Target:", fill=TEXT, font=SANS_LABEL)
    draw_radio(draw, margin+130, y+6, selected=True, label="GS1 firstbase")
    draw_radio(draw, margin+380, y+6, selected=False, label="Swissdamed")
    y += 56

    # Credentials expanded
    draw.text((margin, y), "▼ GS1 firstbase Credentials", fill=ACCENT, font=SANS_LABEL)
    y += 44

    draw.text((margin+20, y+6), "Email:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 500, 42, text="user@example.com")
    y += 56

    draw.text((margin+20, y+6), "Password:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 500, 42, text="••••••••••••")
    y += 56

    draw.text((margin+20, y+6), "Provider GLN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 500, 42, text="7612345000480")
    y += 56

    draw.text((margin+20, y+6), "Publish To GLN:", fill=TEXT, font=SANS_LABEL)
    draw_text_input(draw, margin+230, y, 500, 42, text="7612345000527")
    y += 72

    draw_button(draw, margin, y, 500, 52, "Download, Convert & Push to firstbase", enabled=True, accent=True)
    y += 76

    draw.line([margin, y, W-margin, y], fill=SEPARATOR, width=1)
    y += 16

    draw.text((margin, y), "Log:", fill=TEXT, font=SANS_LABEL)
    y += 40

    draw_log_area(draw, margin, y, W-2*margin, H-y-40, [])

    img.save(os.path.join(OUT_DIR, "screenshot_5_firstbase_creds.png"))
    print("Saved screenshot_5_firstbase_creds.png")


if __name__ == "__main__":
    os.makedirs(OUT_DIR, exist_ok=True)
    screenshot_1_main()
    screenshot_2_running()
    screenshot_3_done()
    screenshot_4_swissdamed()
    screenshot_5_firstbase_creds()
    print(f"\nAll screenshots saved to {OUT_DIR}/")
    print("Size: 2560x1600 (Retina)")
