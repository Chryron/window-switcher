use crate::app::SwitchAppsState;
use crate::utils::{check_error, get_moinitor_rect, is_light_theme, is_win11};

use anyhow::{Context, Result};
use windows::Win32::{
    Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
    Graphics::{
        Dwm::{
            DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
            DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY,
            DWM_TNP_VISIBLE,
        },
        Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn, CreateSolidBrush,
            DeleteDC, DeleteObject, FillRect, FillRgn, GetDC, ReleaseDC, SelectObject,
            SetStretchBltMode, StretchBlt, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HALFTONE,
            HBITMAP, HDC, HPALETTE, SRCCOPY,
        },
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateBitmapFromHBITMAP,
            GdipCreateFont, GdipCreateFontFamilyFromName, GdipCreateFromHDC, GdipCreatePath,
            GdipCreatePen1, GdipCreateSolidFill, GdipCreateStringFormat, GdipDeleteBrush,
            GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics, GdipDeletePath,
            GdipDeletePen, GdipDeleteStringFormat, GdipDisposeImage, GdipDrawImageRect,
            GdipDrawLine, GdipDrawRectangle, GdipDrawString, GdipFillPath, GdipFillRectangle,
            GdipGetPenBrushFill, GdipSetInterpolationMode, GdipSetSmoothingMode,
            GdipSetStringFormatAlign, GdipSetStringFormatLineAlign, GdipSetTextRenderingHint,
            GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpBitmap, GpBrush, GpFont,
            GpFontFamily, GpGraphics, GpImage, GpPath, GpPen, GpSolidFill, GpStringFormat,
            InterpolationModeHighQualityBicubic, SmoothingModeAntiAlias, StringAlignmentCenter,
            TextRenderingHintClearTypeGridFit, Unit,
        },
    },
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{
            DrawIconEx, GetCursorPos, GetWindowRect, SetWindowPos, ShowWindow,
            UpdateLayeredWindow, DI_NORMAL, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, SW_SHOWNA, ULW_ALPHA,
        },
    },
};

pub const BG_DARK_COLOR: u32 = 0x141414;  // Even darker gray (near black)
pub const FG_DARK_COLOR: u32 = 0x242424;  // Dark for foreground elements
pub const BG_LIGHT_COLOR: u32 = 0xe0e0e0;
pub const FG_LIGHT_COLOR: u32 = 0xf2f2f2;
pub const ALPHA_MASK: u32 = 0xff000000;
pub const ICON_SIZE: i32 = 64;
pub const WINDOW_BORDER_SIZE: i32 = 10;
pub const ICON_BORDER_SIZE: i32 = 4;
pub const SCALE_FACTOR: i32 = 6;

// Preview window constants - larger size for better quality
pub const PREVIEW_THUMBNAIL_WIDTH: i32 = 280;
pub const PREVIEW_THUMBNAIL_HEIGHT: i32 = 180;
pub const PREVIEW_PADDING: i32 = 14;
pub const PREVIEW_BORDER_SIZE: i32 = 6;
pub const PREVIEW_TITLE_HEIGHT: i32 = 28;
pub const SELECTION_BORDER_COLOR: u32 = 0x0078D4; // Windows accent blue

// GDI Antialiasing Painter
pub struct GdiAAPainter {
    token: usize,
    hwnd: HWND,
    hdc_screen: HDC,
    rounded_corner: bool,
    show: bool,
}

impl GdiAAPainter {
    pub fn new(hwnd: HWND) -> Result<Self> {
        let startup_input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token: usize = 0;
        check_error(|| unsafe { GdiplusStartup(&mut token, &startup_input, std::ptr::null_mut()) })
            .context("Failed to initialize GDI+")?;

        let hdc_screen = unsafe { GetDC(Some(hwnd)) };
        let rounded_corner = is_win11();

        Ok(Self {
            token,
            hwnd,
            hdc_screen,
            rounded_corner,
            show: false,
        })
    }

    pub fn paint(&mut self, state: &SwitchAppsState) {
        let Coordinate {
            x,
            y,
            width,
            height,
            icon_size,
            item_size,
        } = Coordinate::new(state.apps.len() as i32);

        let corner_radius = if self.rounded_corner {
            item_size / 4
        } else {
            0
        };

        let hwnd = self.hwnd;
        let hdc_screen = self.hdc_screen;

        let (fg_color, bg_color) = theme_color(is_light_theme());

        unsafe {
            let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
            let bitmap_mem = CreateCompatibleBitmap(hdc_screen, width, height);
            SelectObject(hdc_mem, bitmap_mem.into());

            let mut graphics = GpGraphics::default();
            let mut graphics_ptr: *mut GpGraphics = &mut graphics;
            GdipCreateFromHDC(hdc_mem, &mut graphics_ptr as _);
            GdipSetSmoothingMode(graphics_ptr, SmoothingModeAntiAlias);
            GdipSetInterpolationMode(graphics_ptr, InterpolationModeHighQualityBicubic);

            let mut bg_pen = GpPen::default();
            let mut bg_pen_ptr: *mut GpPen = &mut bg_pen;
            GdipCreatePen1(ALPHA_MASK | bg_color, 0.0, Unit(0), &mut bg_pen_ptr as _);

            let mut bg_brush = GpBrush::default();
            let mut bg_brush_ptr: *mut GpBrush = &mut bg_brush;
            GdipGetPenBrushFill(bg_pen_ptr, &mut bg_brush_ptr as _);

            if self.rounded_corner {
                draw_round_rect(
                    graphics_ptr,
                    bg_brush_ptr,
                    0.0,
                    0.0,
                    width as f32,
                    height as f32,
                    corner_radius as f32,
                );
            } else {
                GdipFillRectangle(
                    graphics_ptr,
                    bg_brush_ptr,
                    0.0,
                    0.0,
                    width as f32,
                    height as f32,
                );
            }

            let icons_width = item_size * state.apps.len() as i32;
            let icons_height = item_size;
            let bitmap_icons = draw_icons(
                state,
                hdc_screen,
                icon_size,
                icons_width,
                icons_height,
                corner_radius,
                fg_color,
                bg_color,
            );

            let mut bitmap = GpBitmap::default();
            let mut bitmap_ptr: *mut GpBitmap = &mut bitmap as _;
            GdipCreateBitmapFromHBITMAP(bitmap_icons, HPALETTE::default(), &mut bitmap_ptr as _);

            let image_ptr: *mut GpImage = bitmap_ptr as *mut GpImage;
            GdipDrawImageRect(
                graphics_ptr,
                image_ptr,
                WINDOW_BORDER_SIZE as f32,
                WINDOW_BORDER_SIZE as f32,
                icons_width as f32,
                icons_height as f32,
            );

            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as _,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as _,
                ..Default::default()
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(hdc_screen),
                Some(&POINT { x, y }),
                Some(&SIZE {
                    cx: width,
                    cy: height,
                }),
                Some(hdc_mem),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );

            GdipDisposeImage(image_ptr);
            GdipDeleteBrush(bg_brush_ptr);
            GdipDeletePen(bg_pen_ptr);
            GdipDeleteGraphics(graphics_ptr);

            let _ = DeleteObject(bitmap_icons.into());
            let _ = DeleteObject(bitmap_mem.into());
            let _ = DeleteDC(hdc_mem);
        }

        if self.show {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetFocus(Some(self.hwnd));
        }
        self.show = true;
    }

    pub fn unpaint(&mut self, _state: SwitchAppsState) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.show = false;
    }
}

impl Drop for GdiAAPainter {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(Some(self.hwnd), self.hdc_screen);
            GdiplusShutdown(self.token);
        }
    }
}

pub fn find_clicked_app_index(state: &SwitchAppsState) -> Option<usize> {
    let Coordinate {
        x, y, item_size, ..
    } = Coordinate::new(state.apps.len() as i32);

    let mut cursor_pos = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor_pos) };

    let xpos = cursor_pos.x - x;
    let ypos = cursor_pos.y - y;

    let cy = WINDOW_BORDER_SIZE;
    for (i, _) in state.apps.iter().enumerate() {
        let cx = WINDOW_BORDER_SIZE + item_size * (i as i32);
        if xpos >= cx && xpos < cx + item_size && ypos >= cy && ypos < cy + item_size {
            return Some(i);
        }
    }
    None
}

const fn theme_color(light_theme: bool) -> (u32, u32) {
    match light_theme {
        true => (FG_LIGHT_COLOR, BG_LIGHT_COLOR),
        false => (FG_DARK_COLOR, BG_DARK_COLOR),
    }
}

unsafe fn draw_round_rect(
    graphic_ptr: *mut GpGraphics,
    brush_ptr: *mut GpBrush,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    corner_radius: f32,
) {
    unsafe {
        let mut path = GpPath::default();
        let mut path_ptr: *mut GpPath = &mut path;
        GdipCreatePath(FillModeAlternate, &mut path_ptr as _);
        GdipAddPathArc(
            path_ptr,
            left,
            top,
            corner_radius,
            corner_radius,
            180.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            right - corner_radius,
            top,
            corner_radius,
            corner_radius,
            270.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            right - corner_radius,
            bottom - corner_radius,
            corner_radius,
            corner_radius,
            0.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            left,
            bottom - corner_radius,
            corner_radius,
            corner_radius,
            90.0,
            90.0,
        );
        GdipClosePathFigure(path_ptr);
        GdipFillPath(graphic_ptr, brush_ptr, path_ptr);
        GdipDeletePath(path_ptr);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_icons(
    state: &SwitchAppsState,
    hdc_screen: HDC,
    icon_size: i32,
    width: i32,
    height: i32,
    corner_radius: i32,
    fg_color: u32,
    bg_color: u32,
) -> HBITMAP {
    let scaled_width = width * SCALE_FACTOR;
    let scaled_height = height * SCALE_FACTOR;
    let scaled_corner_radius = corner_radius * SCALE_FACTOR;
    let scaled_border_size = ICON_BORDER_SIZE * SCALE_FACTOR;
    let scaled_icon_inner_size = icon_size * SCALE_FACTOR;
    let scaled_icon_outer_size = scaled_icon_inner_size + scaled_border_size * 2;

    unsafe {
        let hdc_tmp = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_tmp = CreateCompatibleBitmap(hdc_screen, width, height);
        SelectObject(hdc_tmp, bitmap_tmp.into());

        let hdc_scaled = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_scaled = CreateCompatibleBitmap(hdc_screen, scaled_width, scaled_height);
        SelectObject(hdc_scaled, bitmap_scaled.into());

        let fg_brush = CreateSolidBrush(COLORREF(fg_color));
        let bg_brush = CreateSolidBrush(COLORREF(bg_color));

        let rect = RECT {
            left: 0,
            top: 0,
            right: scaled_width,
            bottom: scaled_height,
        };

        FillRect(hdc_scaled, &rect, bg_brush);

        for (i, (icon, _)) in state.apps.iter().enumerate() {
            // draw the box for selected icon
            if i == state.index {
                let left = scaled_icon_outer_size * (i as i32);
                let top = 0;
                let right = left + scaled_icon_outer_size;
                let bottom = top + scaled_icon_outer_size;
                let rgn = CreateRoundRectRgn(
                    left,
                    top,
                    right,
                    bottom,
                    scaled_corner_radius,
                    scaled_corner_radius,
                );
                let _ = FillRgn(hdc_scaled, rgn, fg_brush);
                let _ = DeleteObject(rgn.into());
            }

            let cx = scaled_border_size + scaled_icon_outer_size * (i as i32);
            let _ = DrawIconEx(
                hdc_scaled,
                cx,
                scaled_border_size,
                *icon,
                scaled_icon_inner_size,
                scaled_icon_inner_size,
                0,
                None,
                DI_NORMAL,
            );
        }

        SetStretchBltMode(hdc_tmp, HALFTONE);
        let _ = StretchBlt(
            hdc_tmp,
            0,
            0,
            width,
            height,
            Some(hdc_scaled),
            0,
            0,
            scaled_width,
            scaled_height,
            SRCCOPY,
        );

        let _ = DeleteObject(fg_brush.into());
        let _ = DeleteObject(bg_brush.into());
        let _ = DeleteObject(bitmap_scaled.into());
        let _ = DeleteDC(hdc_scaled);
        let _ = DeleteDC(hdc_tmp);

        bitmap_tmp
    }
}

struct Coordinate {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    icon_size: i32,
    item_size: i32,
}

impl Coordinate {
    fn new(num_apps: i32) -> Self {
        let monitor_rect = get_moinitor_rect();
        let monitor_width = monitor_rect.right - monitor_rect.left;
        let monitor_height = monitor_rect.bottom - monitor_rect.top;

        let icon_size = ((monitor_width - 2 * WINDOW_BORDER_SIZE) / num_apps
            - ICON_BORDER_SIZE * 2)
            .min(ICON_SIZE);

        let item_size = icon_size + ICON_BORDER_SIZE * 2;
        let width = item_size * num_apps + WINDOW_BORDER_SIZE * 2;
        let height = item_size + WINDOW_BORDER_SIZE * 2;
        let x = monitor_rect.left + (monitor_width - width) / 2;
        let y = monitor_rect.top + (monitor_height - height) / 2;

        Self {
            x,
            y,
            width,
            height,
            icon_size,
            item_size,
        }
    }
}

/// State for window preview thumbnails
#[derive(Debug)]
pub struct SwitchWindowsPreviewState {
    pub windows: Vec<(HWND, String)>, // (hwnd, title)
    pub index: usize,
    pub thumbnail_handles: Vec<isize>, // DWM thumbnail handles
    pub hovered_index: Option<usize>,  // Which preview is being hovered
}

impl SwitchWindowsPreviewState {
    pub fn new(windows: Vec<(HWND, String)>, index: usize) -> Self {
        Self {
            windows,
            index,
            thumbnail_handles: Vec::new(),
            hovered_index: None,
        }
    }
}

// Constants for close button
const CLOSE_BUTTON_SIZE: i32 = 20;
const CLOSE_BUTTON_MARGIN: i32 = 4;

struct PreviewCoordinate {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    item_width: i32,
    item_height: i32,
}

impl PreviewCoordinate {
    fn new(num_windows: i32) -> Self {
        let monitor_rect = get_moinitor_rect();
        let monitor_width = monitor_rect.right - monitor_rect.left;
        let monitor_height = monitor_rect.bottom - monitor_rect.top;

        // Calculate thumbnail size based on number of windows
        let max_width = monitor_width - 100;
        let item_width = PREVIEW_THUMBNAIL_WIDTH + PREVIEW_PADDING * 2;
        let item_height = PREVIEW_THUMBNAIL_HEIGHT + PREVIEW_TITLE_HEIGHT + PREVIEW_PADDING * 2;

        // Calculate how many can fit in a row
        let items_per_row = ((max_width - PREVIEW_PADDING * 2) / item_width).max(1).min(num_windows);
        let rows = (num_windows + items_per_row - 1) / items_per_row;

        let width = items_per_row * item_width + PREVIEW_PADDING * 2;
        let height = rows * item_height + PREVIEW_PADDING * 2;

        let x = monitor_rect.left + (monitor_width - width) / 2;
        let y = monitor_rect.top + (monitor_height - height) / 2;

        Self {
            x,
            y,
            width,
            height,
            item_width,
            item_height,
        }
    }

    fn get_item_rect(&self, index: i32, _num_windows: i32) -> RECT {
        let items_per_row = ((self.width - PREVIEW_PADDING * 2) / self.item_width).max(1);
        let row = index / items_per_row;
        let col = index % items_per_row;

        let left = PREVIEW_PADDING + col * self.item_width + PREVIEW_PADDING;
        let top = PREVIEW_PADDING + row * self.item_height + PREVIEW_PADDING;

        RECT {
            left,
            top,
            right: left + PREVIEW_THUMBNAIL_WIDTH,
            bottom: top + PREVIEW_THUMBNAIL_HEIGHT,
        }
    }
}

impl GdiAAPainter {
    /// Paint window preview thumbnails using DWM
    pub fn paint_window_preview(&mut self, state: &mut SwitchWindowsPreviewState) {
        let num_windows = state.windows.len() as i32;
        if num_windows == 0 {
            return;
        }

        let coord = PreviewCoordinate::new(num_windows);
        let corner_radius = if self.rounded_corner { 12 } else { 0 };

        let hwnd = self.hwnd;
        let hdc_screen = self.hdc_screen;
        let is_light = is_light_theme();
        let (fg_color, bg_color) = theme_color(is_light);
        let text_color_argb = if is_light { 0xFF000000u32 } else { 0xFFFFFFFFu32 };

        // First, just position the window (don't show yet - layered window needs content first)
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                coord.x,
                coord.y,
                coord.width,
                coord.height,
                SWP_NOACTIVATE,  // Don't show yet
            );
        }

        unsafe {
            let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
            let bitmap_mem = CreateCompatibleBitmap(hdc_screen, coord.width, coord.height);
            SelectObject(hdc_mem, bitmap_mem.into());

            let mut graphics = GpGraphics::default();
            let mut graphics_ptr: *mut GpGraphics = &mut graphics;
            GdipCreateFromHDC(hdc_mem, &mut graphics_ptr as _);
            GdipSetSmoothingMode(graphics_ptr, SmoothingModeAntiAlias);
            GdipSetInterpolationMode(graphics_ptr, InterpolationModeHighQualityBicubic);

            // Draw background
            let mut bg_pen = GpPen::default();
            let mut bg_pen_ptr: *mut GpPen = &mut bg_pen;
            GdipCreatePen1(ALPHA_MASK | bg_color, 0.0, Unit(0), &mut bg_pen_ptr as _);

            let mut bg_brush = GpBrush::default();
            let mut bg_brush_ptr: *mut GpBrush = &mut bg_brush;
            GdipGetPenBrushFill(bg_pen_ptr, &mut bg_brush_ptr as _);

            if self.rounded_corner {
                draw_round_rect(
                    graphics_ptr,
                    bg_brush_ptr,
                    0.0,
                    0.0,
                    coord.width as f32,
                    coord.height as f32,
                    corner_radius as f32,
                );
            } else {
                GdipFillRectangle(
                    graphics_ptr,
                    bg_brush_ptr,
                    0.0,
                    0.0,
                    coord.width as f32,
                    coord.height as f32,
                );
            }

            // Draw placeholder rectangles for each window and selection highlight
            let mut fg_pen = GpPen::default();
            let mut fg_pen_ptr: *mut GpPen = &mut fg_pen;
            GdipCreatePen1(ALPHA_MASK | fg_color, 0.0, Unit(0), &mut fg_pen_ptr as _);

            let mut fg_brush = GpBrush::default();
            let mut fg_brush_ptr: *mut GpBrush = &mut fg_brush;
            GdipGetPenBrushFill(fg_pen_ptr, &mut fg_brush_ptr as _);

            // Selection highlight color (blue)
            let mut sel_pen = GpPen::default();
            let mut sel_pen_ptr: *mut GpPen = &mut sel_pen;
            GdipCreatePen1(ALPHA_MASK | SELECTION_BORDER_COLOR, 3.0, Unit(0), &mut sel_pen_ptr as _);

            // Hover highlight brush (brighter than foreground)
            let hover_color = if is_light { 0xD8D8D8u32 } else { 0x505050u32 };
            let mut hover_pen = GpPen::default();
            let mut hover_pen_ptr: *mut GpPen = &mut hover_pen;
            GdipCreatePen1(ALPHA_MASK | hover_color, 0.0, Unit(0), &mut hover_pen_ptr as _);
            let mut hover_brush = GpBrush::default();
            let mut hover_brush_ptr: *mut GpBrush = &mut hover_brush;
            GdipGetPenBrushFill(hover_pen_ptr, &mut hover_brush_ptr as _);

            // Close button colors
            let close_bg_color = 0xE81123u32; // Red for close button
            let mut close_pen = GpPen::default();
            let mut close_pen_ptr: *mut GpPen = &mut close_pen;
            GdipCreatePen1(ALPHA_MASK | close_bg_color, 0.0, Unit(0), &mut close_pen_ptr as _);
            let mut close_brush = GpBrush::default();
            let mut close_brush_ptr: *mut GpBrush = &mut close_brush;
            GdipGetPenBrushFill(close_pen_ptr, &mut close_brush_ptr as _);

            // White pen for close X
            let mut x_pen = GpPen::default();
            let mut x_pen_ptr: *mut GpPen = &mut x_pen;
            GdipCreatePen1(0xFFFFFFFF, 2.0, Unit(0), &mut x_pen_ptr as _);

            for (i, (_hwnd, _title)) in state.windows.iter().enumerate() {
                let item_rect = coord.get_item_rect(i as i32, num_windows);
                let is_hovered = state.hovered_index == Some(i);

                // Draw background for thumbnail area (brighter if hovered)
                let brush_to_use = if is_hovered { hover_brush_ptr } else { fg_brush_ptr };
                GdipFillRectangle(
                    graphics_ptr,
                    brush_to_use,
                    (item_rect.left - PREVIEW_BORDER_SIZE) as f32,
                    (item_rect.top - PREVIEW_BORDER_SIZE) as f32,
                    (PREVIEW_THUMBNAIL_WIDTH + PREVIEW_BORDER_SIZE * 2) as f32,
                    (PREVIEW_THUMBNAIL_HEIGHT + PREVIEW_TITLE_HEIGHT + PREVIEW_BORDER_SIZE * 2) as f32,
                );

                // Draw selection highlight for current index
                if i == state.index {
                    GdipDrawRectangle(
                        graphics_ptr,
                        sel_pen_ptr,
                        (item_rect.left - PREVIEW_BORDER_SIZE - 2) as f32,
                        (item_rect.top - PREVIEW_BORDER_SIZE - 2) as f32,
                        (PREVIEW_THUMBNAIL_WIDTH + PREVIEW_BORDER_SIZE * 2 + 4) as f32,
                        (PREVIEW_THUMBNAIL_HEIGHT + PREVIEW_TITLE_HEIGHT + PREVIEW_BORDER_SIZE * 2 + 4) as f32,
                    );
                }

                // Draw close button if hovered
                if is_hovered {
                    let close_x = (item_rect.right - CLOSE_BUTTON_SIZE - CLOSE_BUTTON_MARGIN) as f32;
                    let close_y = (item_rect.top + CLOSE_BUTTON_MARGIN) as f32;
                    let close_size = CLOSE_BUTTON_SIZE as f32;

                    // Draw close button background (red circle/rounded rect)
                    GdipFillRectangle(
                        graphics_ptr,
                        close_brush_ptr,
                        close_x,
                        close_y,
                        close_size,
                        close_size,
                    );

                    // Draw X
                    let padding = 5.0;
                    // Line from top-left to bottom-right
                    GdipDrawLine(
                        graphics_ptr,
                        x_pen_ptr,
                        close_x + padding,
                        close_y + padding,
                        close_x + close_size - padding,
                        close_y + close_size - padding,
                    );
                    // Line from top-right to bottom-left
                    GdipDrawLine(
                        graphics_ptr,
                        x_pen_ptr,
                        close_x + close_size - padding,
                        close_y + padding,
                        close_x + padding,
                        close_y + close_size - padding,
                    );
                }
            }

            GdipDeleteBrush(fg_brush_ptr);
            GdipDeletePen(fg_pen_ptr);
            GdipDeletePen(sel_pen_ptr);
            GdipDeleteBrush(hover_brush_ptr);
            GdipDeletePen(hover_pen_ptr);
            GdipDeleteBrush(close_brush_ptr);
            GdipDeletePen(close_pen_ptr);
            GdipDeletePen(x_pen_ptr);

            // Draw window titles using GDI+ (properly handles alpha on layered windows)
            // Set text rendering hint for ClearType - best quality
            GdipSetTextRenderingHint(graphics_ptr, TextRenderingHintClearTypeGridFit);

            // Create GDI+ font family - use Segoe UI Semibold for better visibility
            let font_name: Vec<u16> = "Segoe UI Semibold\0".encode_utf16().collect();
            let mut font_family: *mut GpFontFamily = std::ptr::null_mut();
            GdipCreateFontFamilyFromName(
                windows::core::PCWSTR(font_name.as_ptr()),
                std::ptr::null_mut(),
                &mut font_family,
            );

            // If Semibold not available, fall back to regular Segoe UI
            if font_family.is_null() {
                let font_name_fallback: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
                GdipCreateFontFamilyFromName(
                    windows::core::PCWSTR(font_name_fallback.as_ptr()),
                    std::ptr::null_mut(),
                    &mut font_family,
                );
            }

            // 13pt font with Bold style for better visibility
            let mut gp_font: *mut GpFont = std::ptr::null_mut();
            GdipCreateFont(
                font_family,
                13.0,  // 13pt font size - slightly bigger
                1,     // FontStyleBold - makes text wider/bolder
                Unit(2), // UnitPoint - DPI-independent
                &mut gp_font,
            );

            // Create text brushes - outline for contrast, main for visibility
            let outline_color = if is_light { 0xFF000000_u32 } else { 0xFF000000_u32 }; // Solid black outline
            let mut outline_brush: *mut GpSolidFill = std::ptr::null_mut();
            GdipCreateSolidFill(outline_color, &mut outline_brush);

            let mut text_brush: *mut GpSolidFill = std::ptr::null_mut();
            GdipCreateSolidFill(text_color_argb, &mut text_brush);

            // Create string format for centered text
            let mut string_format: *mut GpStringFormat = std::ptr::null_mut();
            GdipCreateStringFormat(0, 0, &mut string_format);
            GdipSetStringFormatAlign(string_format, StringAlignmentCenter);
            GdipSetStringFormatLineAlign(string_format, StringAlignmentCenter);

            // Outline offsets - draw text at 8 positions around center for outline effect
            let outline_offsets: [(f32, f32); 8] = [
                (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0),
                (-1.0,  0.0),              (1.0,  0.0),
                (-1.0,  1.0), (0.0,  1.0), (1.0,  1.0),
            ];

            for (i, (_hwnd, title)) in state.windows.iter().enumerate() {
                let item_rect = coord.get_item_rect(i as i32, num_windows);

                // Create layout rect for title (below thumbnail)
                let layout_rect = windows::Win32::Graphics::GdiPlus::RectF {
                    X: (item_rect.left - PREVIEW_BORDER_SIZE) as f32,
                    Y: (item_rect.bottom + 2) as f32,
                    Width: (PREVIEW_THUMBNAIL_WIDTH + PREVIEW_BORDER_SIZE * 2) as f32,
                    Height: (PREVIEW_TITLE_HEIGHT - 4) as f32,
                };

                // Truncate title if too long and add ellipsis
                let display_title = if title.len() > 40 {
                    format!("{}...", &title[..37])
                } else {
                    title.clone()
                };

                let title_wide: Vec<u16> = display_title.encode_utf16().collect();

                // Draw outline (black text at 8 positions around the main text)
                for (dx, dy) in outline_offsets.iter() {
                    let outline_rect = windows::Win32::Graphics::GdiPlus::RectF {
                        X: layout_rect.X + dx,
                        Y: layout_rect.Y + dy,
                        Width: layout_rect.Width,
                        Height: layout_rect.Height,
                    };
                    GdipDrawString(
                        graphics_ptr,
                        windows::core::PCWSTR(title_wide.as_ptr()),
                        title_wide.len() as i32,
                        gp_font,
                        &outline_rect,
                        string_format,
                        outline_brush as *mut GpBrush,
                    );
                }

                // Draw main text on top (white/bright)
                GdipDrawString(
                    graphics_ptr,
                    windows::core::PCWSTR(title_wide.as_ptr()),
                    title_wide.len() as i32,
                    gp_font,
                    &layout_rect,
                    string_format,
                    text_brush as *mut GpBrush,
                );
            }

            // Cleanup GDI+ text resources
            GdipDeleteStringFormat(string_format);
            GdipDeleteBrush(outline_brush as *mut GpBrush);
            GdipDeleteBrush(text_brush as *mut GpBrush);
            GdipDeleteFont(gp_font);
            GdipDeleteFontFamily(font_family);

            // Update layered window
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as _,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as _,
                ..Default::default()
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(hdc_screen),
                Some(&POINT { x: coord.x, y: coord.y }),
                Some(&SIZE {
                    cx: coord.width,
                    cy: coord.height,
                }),
                Some(hdc_mem),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );

            GdipDeleteBrush(bg_brush_ptr);
            GdipDeletePen(bg_pen_ptr);
            GdipDeleteGraphics(graphics_ptr);

            let _ = DeleteObject(bitmap_mem.into());
            let _ = DeleteDC(hdc_mem);
        }

        // Now register DWM thumbnails
        self.unregister_thumbnails(state);
        
        for (i, (src_hwnd, _title)) in state.windows.iter().enumerate() {
            let item_rect = coord.get_item_rect(i as i32, num_windows);
            
            // Convert to screen coordinates
            let dest_rect = RECT {
                left: item_rect.left,
                top: item_rect.top,
                right: item_rect.right,
                bottom: item_rect.bottom,
            };

            if let Ok(thumbnail) = self.register_thumbnail(*src_hwnd, dest_rect) {
                state.thumbnail_handles.push(thumbnail);
            }
        }

        // Now show the window (after content is set via UpdateLayeredWindow and thumbnails registered)
        unsafe {
            // Always ensure the window is visible and on top
            let _ = ShowWindow(self.hwnd, SW_SHOWNA);  // Show without activating
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                coord.x,
                coord.y,
                coord.width,
                coord.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.show = true;
    }

    fn register_thumbnail(&self, src_hwnd: HWND, dest_rect: RECT) -> Result<isize> {
        unsafe {
            let thumbnail = DwmRegisterThumbnail(self.hwnd, src_hwnd)
                .context("Failed to register thumbnail")?;

            // Get source window size to calculate aspect ratio
            let mut src_rect = RECT::default();
            let _ = GetWindowRect(src_hwnd, &mut src_rect);
            let src_width = src_rect.right - src_rect.left;
            let src_height = src_rect.bottom - src_rect.top;

            // Calculate destination rect maintaining aspect ratio
            let dest_width = dest_rect.right - dest_rect.left;
            let dest_height = dest_rect.bottom - dest_rect.top;

            let (final_width, final_height) = if src_width > 0 && src_height > 0 {
                let src_aspect = src_width as f32 / src_height as f32;
                let dest_aspect = dest_width as f32 / dest_height as f32;

                if src_aspect > dest_aspect {
                    // Source is wider, fit to width
                    let h = (dest_width as f32 / src_aspect) as i32;
                    (dest_width, h)
                } else {
                    // Source is taller, fit to height
                    let w = (dest_height as f32 * src_aspect) as i32;
                    (w, dest_height)
                }
            } else {
                (dest_width, dest_height)
            };

            // Center the thumbnail in the destination area
            let x_offset = (dest_width - final_width) / 2;
            let y_offset = (dest_height - final_height) / 2;

            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
                rcDestination: RECT {
                    left: dest_rect.left + x_offset,
                    top: dest_rect.top + y_offset,
                    right: dest_rect.left + x_offset + final_width,
                    bottom: dest_rect.top + y_offset + final_height,
                },
                rcSource: RECT::default(),
                opacity: 255,
                fVisible: true.into(),
                fSourceClientAreaOnly: true.into(),
            };

            DwmUpdateThumbnailProperties(thumbnail, &props)
                .context("Failed to update thumbnail properties")?;

            Ok(thumbnail)
        }
    }

    fn unregister_thumbnails(&self, state: &mut SwitchWindowsPreviewState) {
        for thumbnail in state.thumbnail_handles.drain(..) {
            unsafe {
                let _ = DwmUnregisterThumbnail(thumbnail);
            }
        }
    }

    /// Just cleanup thumbnails without hiding window (for when cycling)
    pub fn cleanup_thumbnails_only(&mut self, mut state: SwitchWindowsPreviewState) {
        self.unregister_thumbnails(&mut state);
    }

    /// Hide and cleanup window preview
    pub fn unpaint_window_preview(&mut self, mut state: SwitchWindowsPreviewState) {
        self.unregister_thumbnails(&mut state);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.show = false;
    }
}

pub fn find_clicked_preview_index(state: &SwitchWindowsPreviewState) -> Option<usize> {
    let num_windows = state.windows.len() as i32;
    if num_windows == 0 {
        return None;
    }

    let coord = PreviewCoordinate::new(num_windows);

    let mut cursor_pos = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor_pos) };

    let xpos = cursor_pos.x - coord.x;
    let ypos = cursor_pos.y - coord.y;

    for i in 0..num_windows {
        let item_rect = coord.get_item_rect(i, num_windows);
        let padded_rect = RECT {
            left: item_rect.left - PREVIEW_BORDER_SIZE,
            top: item_rect.top - PREVIEW_BORDER_SIZE,
            right: item_rect.right + PREVIEW_BORDER_SIZE,
            bottom: item_rect.bottom + PREVIEW_TITLE_HEIGHT + PREVIEW_BORDER_SIZE,
        };

        if xpos >= padded_rect.left
            && xpos < padded_rect.right
            && ypos >= padded_rect.top
            && ypos < padded_rect.bottom
        {
            return Some(i as usize);
        }
    }
    None
}

/// Find which preview the mouse is hovering over (same as clicked but for hover tracking)
pub fn find_hovered_preview_index(state: &SwitchWindowsPreviewState) -> Option<usize> {
    find_clicked_preview_index(state)
}

/// Check if the close button was clicked for a specific preview
pub fn is_close_button_clicked(state: &SwitchWindowsPreviewState, preview_index: usize) -> bool {
    let num_windows = state.windows.len() as i32;
    if num_windows == 0 || preview_index >= state.windows.len() {
        return false;
    }

    let coord = PreviewCoordinate::new(num_windows);

    let mut cursor_pos = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor_pos) };

    let xpos = cursor_pos.x - coord.x;
    let ypos = cursor_pos.y - coord.y;

    let item_rect = coord.get_item_rect(preview_index as i32, num_windows);
    
    // Close button is in top-right corner of the preview
    let close_rect = RECT {
        left: item_rect.right - CLOSE_BUTTON_SIZE - CLOSE_BUTTON_MARGIN,
        top: item_rect.top + CLOSE_BUTTON_MARGIN,
        right: item_rect.right - CLOSE_BUTTON_MARGIN,
        bottom: item_rect.top + CLOSE_BUTTON_SIZE + CLOSE_BUTTON_MARGIN,
    };

    xpos >= close_rect.left
        && xpos < close_rect.right
        && ypos >= close_rect.top
        && ypos < close_rect.bottom
}
