// Ruiss 程序入口。
// 不启用 Windows console（release 下），托盘常驻。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ruiss_lib::run()
}
