# 中文字体

`NotoSansCJKsc-Regular.otf` 是 Noto Sans CJK 简体中文常规字重字体。
使用未修改的静态 OTF 字体，在无系统中文字体的机器上也能离线显示聊天。
保留完整字库会增加约 16.4 MB 的程序体积。静态常规字重避免可变字体在
当前字体渲染器中默认落到过细字重，影响小字号可读性。

- 来源：[notofonts/noto-cjk](https://github.com/notofonts/noto-cjk/blob/165c01b46ea533872e002e0785ff17e44f6d97d8/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf)
- 固定提交：`165c01b46ea533872e002e0785ff17e44f6d97d8`
- SHA-256：`2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`
- 许可证：随附 [OFL.txt](OFL.txt)，SIL Open Font License 1.1，条款来自同一提交的 [LICENSE](https://github.com/notofonts/noto-cjk/blob/165c01b46ea533872e002e0785ff17e44f6d97d8/LICENSE)。原始版权信息保留在未修改字体的元数据中。

程序通过 `include_bytes!` 内嵌字体，运行时不下载字体。
