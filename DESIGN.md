---
version: alpha
colors:
  background: '#18242E'
  surface: '#213440'
  text: '#ECF3F7'
  muted: '#ADC1CC'
  primary: '#82C8E8'
  danger: '#FFB0AC'
typography:
  sans:
    fontFamily: 'PingFang SC, Microsoft YaHei, Noto Sans CJK SC, sans-serif'
    fontSize: '16px'
  mono:
    fontFamily: 'Hack, monospace'
    fontSize: '13px'
rounded:
  DEFAULT: '6px'
spacing:
  unit: '8px'
components:
  button:
    height: '32px'
    textColor: '{colors.text}'
  secondaryLabel:
    textColor: '{colors.muted}'
  errorLabel:
    textColor: '{colors.danger}'
---
# NeonMix 工程诊断界面

## Overview

面向三端音频开发者的设备检查工具。借鉴音频机架的通道标识，以同一设备的输入/输出格式并列呈现为辨识点；页面只负责设备发现和诊断，完整 Sender/Hub 管理在 E07。没有既存界面或 sibling workflow。

## Colors

蓝灰底色表示工作台，浅蓝仅标识可操作控件，错误同时显示文字。唯一运行时映射由 `apps/desktop/src/theme.rs` 维护，对应上述 hex 值。只实现深色工程主题。

## Typography

中文 16 px 正文；设备 ID 采用 egui 内置等宽字。中文字体只读加载系统字体；缺少时显示可执行安装指引。标题 28 px，标签 13 px。

## Layout

单页，上方固定标题和刷新按钮，中间固定高度状态区，下方独立滚动设备列表。520 px 最小窗口宽度；窄窗口格式行自动换行，ID 可以选择复制。间隔 8/16/24 px。

## Elevation & Depth

无投影和动画，面板深浅区分层次。

## Shapes

6 px 圆角继承 egui 原生控件，保证键盘焦点和状态可辨认。

## Components

刷新按钮、只读设备记录和持久状态文本使用 egui 组件。ScrollArea 的滚动条始终可见。后台枚举时禁用刷新，保留上一轮数据并明确标记。

## Do's and Don'ts

显示真实设备数据、具体错误和重新刷新入口；不展示假电平、假连接成功或未实现的播放控件。页面关闭不影响独立运行的音频 CLI。
