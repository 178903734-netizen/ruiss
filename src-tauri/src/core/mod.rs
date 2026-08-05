// core：双端共享逻辑 —— 协议定义、坐标换算、键位映射、控制仲裁。
// 这部分不依赖任何平台 API，Windows / Mac 两端用同一份代码。

pub mod arbiter;
pub mod geometry;
pub mod keys;
pub mod protocol;
