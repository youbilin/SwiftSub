use super::*;
impl SwiftSubApp {
    pub fn share_subscription_qr(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.server_url.clone() {
            self.open_qr("订阅地址 · 仅此电脑可访问".into(), url, cx);
        }
    }
    pub fn share_node_qr(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(node) = self.nodes.iter().find(|n| n.source_key == key) {
            match node.proxy.to_yaml_text() {
                Ok(yaml) => self.open_qr(
                    format!("{} · 节点 YAML", node.name()),
                    format!(
                        "proxies:\n  -\n{}",
                        yaml.lines()
                            .map(|l| format!("    {l}\n"))
                            .collect::<String>()
                    ),
                    cx,
                ),
                Err(e) => self.show_toast(e.to_string(), cx),
            }
        }
    }
    fn open_qr(&mut self, title: String, content: String, cx: &mut Context<Self>) {
        match qrcode::QrCode::new(content.as_bytes()) {
            Ok(_) => {
                self.qr_content = Some((title, content));
                cx.notify();
            }
            Err(e) => self.show_toast(format!("内容过长，无法生成二维码：{e}"), cx),
        }
    }
    pub fn copy_qr_content(&self, cx: &mut Context<Self>) {
        if let Some((_, content)) = &self.qr_content {
            cx.write_to_clipboard(ClipboardItem::new_string(content.clone()));
        }
    }
    pub fn export_qr(&mut self, cx: &mut Context<Self>) {
        let Some((_, content)) = &self.qr_content else {
            return;
        };
        let code = match qrcode::QrCode::new(content.as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                self.show_toast(e.to_string(), cx);
                return;
            }
        };
        let n = code.width() + 8;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {n} {n}\" width=\"512\" height=\"512\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/>"
        );
        for y in 0..code.width() {
            for x in 0..code.width() {
                if code[(x, y)] == qrcode::Color::Dark {
                    svg.push_str(&format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"1\" height=\"1\" fill=\"black\"/>",
                        x + 4,
                        y + 4
                    ));
                }
            }
        }
        svg.push_str("</svg>");
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SVG", &["svg"])
            .set_file_name("swiftsub-qr.svg")
            .save_file()
        {
            match std::fs::write(path, svg) {
                Ok(()) => self.show_toast("二维码已导出".into(), cx),
                Err(e) => self.show_toast(e.to_string(), cx),
            }
        }
    }
}
