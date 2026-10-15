// 发布版用 Windows GUI 子系统：双击 exe 不再弹出控制台窗口。
// debug 版保留控制台，方便看 panic 信息和 eprintln 调试输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use gpui::{
    App, Application, Bounds, ClickEvent, Context, SharedString, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};

use code_count::count::{self, Options, Progress, Report, human_bytes, thousands};

// ---------------------------------------------------------------- 配色

const BG: u32 = 0x1b1d23;
const PANEL: u32 = 0x232630;
const PANEL_ALT: u32 = 0x2b2f3b;
const BORDER: u32 = 0x363b4a;
const FG: u32 = 0xe6e8ee;
const FG_DIM: u32 = 0x9aa1b1;
const ACCENT: u32 = 0x5b9dff;

// ---------------------------------------------------------------- 状态

/// 后台线程 -> UI 线程的消息。
enum ScanMsg {
    Progress(Box<Progress>),
    Done(Box<Report>),
}

struct State {
    root: Option<PathBuf>,
    /// 正在扫描中。
    scanning: bool,
    /// 文件夹选择对话框已弹出。
    picking: bool,
    progress: Option<Progress>,
    report: Option<Report>,
    error: Option<String>,
}

struct CodeCount {
    state: State,
    opts: Options,
}

impl CodeCount {
    fn new(cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            state: State {
                root: None,
                scanning: false,
                picking: false,
                progress: None,
                report: None,
                error: None,
            },
            opts: Options::default(),
        }
    }

    /// 弹出原生「选择文件夹」对话框。
    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        if self.state.picking || self.state.scanning {
            return;
        }
        self.state.picking = true;
        self.state.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("选择要统计的文件夹")
                .pick_folder()
                .await
                .map(|h| h.path().to_path_buf());

            this.update(cx, |this, cx| {
                this.state.picking = false;
                match picked {
                    Some(path) => this.begin_scan(path, cx),
                    None => cx.notify(),
                }
            })
            .ok();
        })
        .detach();
    }

    /// 对已选目录重新跑一次统计。
    fn rescan(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.state.root.clone() {
            self.begin_scan(root, cx);
        }
    }

    /// 启动一次后台统计。
    fn begin_scan(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.state.scanning {
            return;
        }
        self.state.root = Some(root.clone());
        self.state.scanning = true;
        self.state.progress = None;
        self.state.report = None;
        self.state.error = None;
        cx.notify();

        // 无界通道：扫描线程发进度，UI 侧异步接收。
        let (tx, rx) = smol::channel::unbounded::<ScanMsg>();
        let opts = self.opts.clone();

        // 真正干活的地方，跑在 gpui 的后台线程池上，不阻塞 UI。
        cx.background_executor()
            .spawn(async move {
                let progress_tx = tx.clone();
                let report = count::count_dir(&root, &opts, move |p| {
                    // try_send 不阻塞，UI 侧来不及处理时直接丢帧即可。
                    let _ = progress_tx.try_send(ScanMsg::Progress(Box::new(p.clone())));
                });
                let _ = tx.send(ScanMsg::Done(Box::new(report))).await;
            })
            .detach();

        // UI 侧：合并积压的进度消息，只重绘一次。
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut latest = first;
                while let Ok(next) = rx.try_recv() {
                    latest = next;
                }
                let finished = matches!(latest, ScanMsg::Done(_));

                if this
                    .update(cx, |this, cx| {
                        match latest {
                            ScanMsg::Progress(p) => this.state.progress = Some(*p),
                            ScanMsg::Done(r) => {
                                this.state.scanning = false;
                                this.state.progress = None;
                                this.state.report = Some(*r);
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break; // 窗口已关闭
                }
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    // ------------------------------------------------------------ 渲染辅助

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.state.scanning || self.state.picking;

        let pick_label = if self.state.picking {
            "正在选择…"
        } else {
            "选择文件夹"
        };

        let mut buttons = div().flex().flex_row().gap_2().child(
            button("pick", pick_label)
                .primary()
                .enabled(!busy)
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.pick_folder(cx)
                })),
        );

        if self.state.root.is_some() {
            buttons = buttons.child(
                button("rescan", "重新统计")
                    .enabled(!busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.rescan(cx)
                    })),
            );
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(FG))
                            .child("代码行数统计"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(FG_DIM))
                            .child(match &self.state.root {
                                Some(p) => p.display().to_string(),
                                None => "尚未选择文件夹".to_string(),
                            }),
                    ),
            )
            .child(buttons)
    }

    fn body(&self) -> gpui::AnyElement {
        if let Some(err) = &self.state.error {
            return div()
                .p_4()
                .rounded_md()
                .bg(rgb(0x3a2126))
                .border_1()
                .border_color(rgb(0x7a2f3a))
                .text_sm()
                .text_color(rgb(0xff9aa5))
                .child(err.clone())
                .into_any_element();
        }

        if self.state.scanning {
            let (files, lines) = self
                .state
                .progress
                .as_ref()
                .map(|p| (p.files, p.lines))
                .unwrap_or((0, 0));
            let current = self
                .state
                .progress
                .as_ref()
                .map(|p| p.current.display().to_string())
                .unwrap_or_else(|| "正在遍历目录…".to_string());

            return div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_md()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .text_color(rgb(ACCENT))
                        .child("统计中…")
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(FG_DIM))
                                .child(format!(
                                    "已处理 {} 个文件 / {} 行",
                                    thousands(files),
                                    thousands(lines)
                                )),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FG_DIM))
                        .truncate()
                        .child(current),
                )
                .into_any_element();
        }

        let Some(report) = &self.state.report else {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .py_8()
                .text_color(rgb(FG_DIM))
                .child("点击右上角「选择文件夹」开始统计")
                .child(
                    div()
                        .text_sm()
                        .child("会自动跳过 .git / node_modules / target 等目录，并忽略二进制文件"),
                )
                .into_any_element();
        };

        let mut col = div().flex().flex_col().gap_3().w_full();

        // ---- 概览卡片
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .w_full()
                .child(stat_card("总行数", &thousands(report.lines), ACCENT))
                .child(stat_card("文件数", &thousands(report.files), 0x6fd3a0))
                .child(stat_card("总大小", &human_bytes(report.bytes), 0xf0b36b))
                .child(stat_card(
                    "跳过目录",
                    &thousands(report.skipped_dirs),
                    0xa78bfa,
                )),
        );

        // ---- 次级信息
        let mut notes = Vec::new();
        if report.binary_files > 0 {
            notes.push(format!("忽略二进制文件 {} 个", report.binary_files));
        }
        if report.filtered_files > 0 {
            notes.push(format!("按扩展名过滤 {} 个", report.filtered_files));
        }
        if report.errors > 0 {
            notes.push(format!("读取失败 {} 个", report.errors));
        }
        if !notes.is_empty() {
            col = col.child(
                div()
                    .text_sm()
                    .text_color(rgb(FG_DIM))
                    .child(notes.join(" · ")),
            );
        }

        // ---- 按扩展名排行
        let ranked = report.ranked();
        let max_lines = ranked.first().map(|(_, s)| s.lines).unwrap_or(1).max(1);

        let mut table = div().flex().flex_col().w_full().child(table_row(
            "语言 / 扩展名",
            "文件",
            "行数",
            "大小",
            Some("占比"),
            true,
            None,
            None,
        ));

        for (i, (ext, stat)) in ranked.iter().enumerate() {
            table = table.child(table_row(
                if ext.is_empty() { "(无扩展名)" } else { ext },
                &thousands(stat.files),
                &thousands(stat.lines),
                &human_bytes(stat.bytes),
                None,
                false,
                Some(i % 2 == 1),
                Some((
                    stat.lines as f32 / max_lines as f32,
                    stat.lines as f32 / report.lines.max(1) as f32,
                )),
            ));
        }

        col.child(
            div()
                .id("ext-scroll")
                .flex()
                .flex_col()
                .w_full()
                .max_h(px(420.0))
                .rounded_md()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(BORDER))
                .overflow_y_scroll()
                .child(table),
        )
        .into_any_element()
    }
}

impl Render for CodeCount {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(FG))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .w_full()
                    .child(self.header(cx))
                    .child(self.body()),
            )
    }
}

// ---------------------------------------------------------------- 小部件

fn stat_card(label: &str, value: &str, accent: u32) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.0))
        .p_3()
        .rounded_md()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(BORDER))
        .child(div().text_xs().text_color(rgb(FG_DIM)).child(label.to_string()))
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(accent))
                .child(value.to_string()),
        )
}

/// 表头 / 数据行，共用一套列宽以保持对齐。
#[allow(clippy::too_many_arguments)]
fn table_row(
    name: &str,
    files: &str,
    lines: &str,
    bytes: &str,
    percent: Option<&str>,
    is_header: bool,
    alt: Option<bool>,
    bar: Option<(f32, f32)>,
) -> impl IntoElement {
    let cell = |w: f32, text: String, align_right: bool| {
        let mut d = div().w(px(w)).flex_none().text_sm().child(text);
        if align_right {
            d = d.text_right();
        }
        if is_header {
            d = d.text_color(rgb(FG_DIM));
        }
        d
    };

    let bg = if is_header || alt == Some(true) {
        rgb(PANEL_ALT)
    } else {
        rgb(PANEL)
    };

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px_3()
        .py_2()
        .bg(bg)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .truncate()
                .when(is_header, |d| d.text_color(rgb(FG_DIM)))
                .child(name.to_string()),
        )
        .child(cell(90.0, files.to_string(), true))
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_sm()
                .text_right()
                .when(is_header, |d| d.text_color(rgb(FG_DIM)))
                .when(!is_header, |d| {
                    d.font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(ACCENT))
                })
                .child(lines.to_string()),
        )
        .child(cell(100.0, bytes.to_string(), true));

    // 占比列与条形图列：表头和数据行都必须占位，否则列会错位。
    row = row.child(
        div()
            .w(px(70.0))
            .flex_none()
            .text_sm()
            .text_right()
            .text_color(rgb(FG_DIM))
            .child(percent.unwrap_or("").to_string()),
    );

    row = row.child(match bar {
        Some((bar_ratio, label_ratio)) => div()
            .w(px(170.0))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(rgb(0x30343f))
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(bar_ratio.clamp(0.02, 1.0)))
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    ),
            )
            .child(
                div()
                    .w(px(52.0))
                    .flex_none()
                    .text_xs()
                    .text_right()
                    .text_color(rgb(FG_DIM))
                    .child(format!("{:.1}%", label_ratio * 100.0)),
            )
            .into_any_element(),
        None => div().w(px(170.0)).flex_none().into_any_element(),
    });

    row
}

/// 按钮点击回调。
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// 一个朴素的按钮，带 hover / active 反馈。
#[derive(gpui::IntoElement)]
struct Btn {
    id: SharedString,
    label: SharedString,
    primary: bool,
    enabled: bool,
    handler: Option<ClickHandler>,
}

fn button(id: &str, label: &str) -> Btn {
    Btn {
        id: id.to_string().into(),
        label: label.to_string().into(),
        primary: false,
        enabled: true,
        handler: None,
    }
}

impl Btn {
    fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Btn {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (bg, fg, hover_bg) = if !self.enabled {
            (rgb(0x2a2d38), rgb(0x6b7280), rgb(0x2a2d38))
        } else if self.primary {
            (rgb(ACCENT), rgb(0x0d1017), rgb(0x7db2ff))
        } else {
            (rgb(PANEL_ALT), rgb(FG), rgb(0x3a3f4d))
        };

        div()
            .id(self.id)
            .flex_none()
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(bg)
            .text_sm()
            .text_color(fg)
            .child(self.label)
            .when(self.enabled, |d| {
                d.cursor_pointer()
                    .hover(move |this| this.bg(hover_bg))
                    .active(|this| this.opacity(0.85))
            })
            .when_some(self.handler, |d, h| {
                d.on_click(move |event, window, cx| h(event, window, cx))
            })
    }
}

// ---------------------------------------------------------------- main

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.0), px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.0), px(480.0))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("代码行数统计".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| cx.new(CodeCount::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
