#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Disable DMA-BUF renderer to fix GBM buffer errors on some Linux systems
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    ears_lib::run();
}
