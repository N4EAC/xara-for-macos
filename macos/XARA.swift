import AppKit
import Darwin
import AVFoundation
import WebKit

struct StationConfiguration: Codable {
    var call = ""
    var mode = "hf2300"
    var input = "default"
    var output = "default"
    var ptt = "off"
    var pttAddress = ""
    var commandPort = "8300"
    var kissPort = "8100"
    // Optional so older saved configurations decode with external ports disabled.
    var externalPorts: Bool?

    var text: String {
        let modeLine = mode.hasPrefix("hf") ? "hf=\(mode.dropFirst(2))" : "fm=\(mode.dropFirst(2))"
        var method = ptt
        if ["rts", "dtr", "rigctld"].contains(ptt) { method += ":" + pttAddress }
        return "call=\(call)\n\(modeLine)\ncapture=\(input)\nplayback=\(output)\nptt=\(method)\nhost-ports=\(externalPorts == true ? "on" : "off")\nport=\(commandPort)\nkiss-port=\(kissPort)"
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKDownloadDelegate, NSMenuItemValidation, WKScriptMessageHandler, NSWindowDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var modemURL: URL?
    var engineRunning = false
    var fitOnFirstTelemetry = true
    var timer: Timer?
    var closing = false
    var configuration = StationConfiguration()
    let defaults = UserDefaults.standard
    var stateDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("XARA", isDirectory: true)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if let data = defaults.data(forKey: "stationConfiguration"), let saved = try? JSONDecoder().decode(StationConfiguration.self, from: data) { configuration = saved }
        let menu = NSMenu()
        let appItem = NSMenuItem(); menu.addItem(appItem)
        let appMenu = NSMenu(); appItem.submenu = appMenu
        appMenu.addItem(withTitle: "About XARA", action: #selector(about), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit XARA", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        let modemItem = NSMenuItem(); menu.addItem(modemItem)
        let modemMenu = NSMenu(title: "Modem"); modemItem.submenu = modemMenu
        modemMenu.addItem(withTitle: "Station & Radio…", action: #selector(configure), keyEquivalent: ",")
        modemMenu.addItem(withTitle: "Reload", action: #selector(reload), keyEquivalent: "r")
        modemMenu.addItem(withTitle: "Fit to Panels", action: #selector(fitToPanels), keyEquivalent: "0")
        let browserItem = modemMenu.addItem(withTitle: "Open in Browser", action: #selector(openInBrowser), keyEquivalent: "b")
        browserItem.target = self
        browserItem.keyEquivalentModifierMask = [.command, .shift]
        let edit = NSMenuItem(); menu.addItem(edit)
        let editMenu = NSMenu(title: "Edit"); edit.submenu = editMenu
        for (title, selector, key) in [("Cut", "cut:", "x"), ("Copy", "copy:", "c"), ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            editMenu.addItem(withTitle: title, action: Selector(selector), keyEquivalent: key)
        }
        applyMenuFont(menu)
        NSApp.mainMenu = menu
        let webConfig = WKWebViewConfiguration()
        webConfig.websiteDataStore = .nonPersistent()
        webConfig.userContentController.add(self, name: "panelReady")
        web = WKWebView(frame: .zero, configuration: webConfig)
        web.navigationDelegate = self
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "XARA"
        window.minSize = NSSize(width: 720, height: 360)
        window.delegate = self
        window.contentView = web
        window.backgroundColor = NSColor(calibratedRed: 0.063, green: 0.078, blue: 0.086, alpha: 1)
        window.appearance = NSAppearance(named: .darkAqua)
        window.setFrameAutosaveName("XARAWorkspace")
        window.center(); window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        showMessage("Stopped", "Modem → Station & Radio…")
        DispatchQueue.main.async { [self] in
            if configuration.call.isEmpty { configure() } else { start() }
        }
    }

    func savedSettings() -> [String: String] {
        guard let text = try? String(contentsOf: stateDirectory.appendingPathComponent("settings"), encoding: .utf8) else { return [:] }
        var values = [String: String]()
        for line in text.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            if parts.count == 2 && !parts[0].hasPrefix("#") { values[parts[0]] = parts[1] }
        }
        return values
    }

    func deviceChoices(_ kind: String) -> [(String, String)] {
        let count = xara_embedded_devices(nil, 0)
        var text = [CChar](repeating: 0, count: max(count, 1))
        let needed = xara_embedded_devices(&text, text.count)
        guard needed <= text.count,
              let data = String(cString: text).data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let list = object[kind] as? [[String: String]] else { return [] }
        return list.compactMap { d in guard let name = d["name"], let desc = d["desc"] else { return nil }; return (name, desc) }
    }

    @objc func configure() {
        guard xara_embedded_can_configure() != 0 else { alert("Modem is busy", "Disconnect the radio link and stop tuning before changing station hardware."); return }
        let saved = savedSettings()
        var current = configuration
        current.call = saved["call"] ?? current.call
        current.input = saved["capture"] ?? current.input
        current.output = saved["playback"] ?? current.output
        if let hf = saved["hf"] { current.mode = "hf" + hf }
        if let fm = saved["fm"] { current.mode = "fm" + fm }
        let dialog = NSAlert()
        dialog.messageText = "Station & Radio"
        let form = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 490))
        var y: CGFloat = 461
        func add(_ title: String, _ control: NSControl) {
            let label = NSTextField(labelWithString: title)
            label.frame = NSRect(x: 0, y: y + 4, width: 140, height: 20)
            control.frame = NSRect(x: 145, y: y, width: 375, height: 28)
            control.setAccessibilityLabel(title)
            form.addSubview(label); form.addSubview(control); y -= 41
        }
        let call = NSTextField(string: current.call); call.placeholderString = "Callsign"; add("Callsign", call)
        let mode = NSPopUpButton()
        let modes = [("hf500", "HF · 500 Hz"), ("hf2300", "HF · 2300 Hz"), ("hf2750", "HF · 2750 Hz"), ("fmnarrow", "FM · Narrow"), ("fmwide", "FM · Wide")]
        mode.addItems(withTitles: modes.map { $0.1 }); mode.selectItem(at: modes.firstIndex { $0.0 == current.mode } ?? 1); add("Modem mode", mode)
        func deviceMenu(_ kind: String, _ selected: String) -> (NSPopUpButton, [(String, String)]) {
            var devices = [("default", "System default (receive only)")] + deviceChoices(kind)
            if !devices.contains(where: { $0.0 == selected }) { devices.append((selected, "Unavailable device")) }
            let menu = NSPopUpButton(); menu.addItems(withTitles: devices.map { $0.1 }); menu.selectItem(at: devices.firstIndex { $0.0 == selected } ?? 0)
            return (menu, devices)
        }
        let (input, inputs) = deviceMenu("capture", current.input); add("Radio audio input", input)
        let (output, outputs) = deviceMenu("playback", current.output); add("Radio audio output", output)
        let ptt = NSPopUpButton()
        let methods = [("off", "Receive only · transmit disabled"), ("rts", "Serial RTS"), ("dtr", "Serial DTR"), ("rigctld", "Hamlib rigctld"), ("vox", "VOX")]
        ptt.addItems(withTitles: methods.map { $0.1 }); ptt.selectItem(at: methods.firstIndex { $0.0 == current.ptt } ?? 0); add("PTT method", ptt)
        let address = NSTextField(string: current.pttAddress); address.placeholderString = "/dev/cu.usbserial… or 127.0.0.1:4532"; add("PTT device / host", address)
        let ports = NSButton(checkboxWithTitle: "Enable command/data and KISS ports", target: nil, action: nil)
        ports.state = current.externalPorts == true ? .on : .off
        add("External software", ports)
        let port = NSTextField(string: current.commandPort); add("Command port", port)
        let kiss = NSTextField(string: current.kissPort); kiss.placeholderString = "8100 or off"; add("KISS port", kiss)
        let note = NSTextField(wrappingLabelWithString: "Data port = command port + 1. Ports are local and unauthenticated; enable only for trusted radio software.\nSaved locally: station, audio, PTT, and port settings. Received data stays in memory unless exported.")
        note.font = .monospacedSystemFont(ofSize: 11, weight: .regular); note.textColor = .secondaryLabelColor
        note.frame = NSRect(x: 0, y: 0, width: 520, height: 74); form.addSubview(note)
        dialog.accessoryView = form
        dialog.addButton(withTitle: "Start modem"); dialog.addButton(withTitle: "Cancel")
        applyMonospace(dialog.window.contentView)
        dialog.window.initialFirstResponder = call
        guard dialog.runModal() == .alertFirstButtonReturn else { return }
        current.call = call.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        current.mode = modes[mode.indexOfSelectedItem].0
        current.input = inputs[input.indexOfSelectedItem].0; current.output = outputs[output.indexOfSelectedItem].0
        current.ptt = methods[ptt.indexOfSelectedItem].0
        current.pttAddress = address.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        current.externalPorts = ports.state == .on
        current.commandPort = port.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        current.kissPort = kiss.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let validCall = current.call.range(of: "^[A-Z0-9]{1,15}(-[0-9]{1,2})?$", options: .regularExpression) != nil && !current.call.hasPrefix("N0CALL")
        let command = Int(current.commandPort) ?? -1
        let kissNumber = Int(current.kissPort) ?? -1
        let validKiss = current.kissPort == "off" || (1...65535).contains(kissNumber) && kissNumber != command && kissNumber != command + 1
        guard validCall, (1...65534).contains(command), validKiss,
              !current.pttAddress.contains(where: { $0.isNewline }),
              current.ptt == "off" || (current.input != "default" && current.output != "default") else {
            alert("Check station settings", "Enter your station callsign, distinct valid TCP ports, and explicit radio audio devices when transmit is enabled."); return
        }
        configuration = current
        stop { [self] in
            do {
                try FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let attributes = try FileManager.default.attributesOfItem(atPath: stateDirectory.path)
                guard attributes[.type] as? FileAttributeType == .typeDirectory else { throw POSIXError(.EINVAL) }
                try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: stateDirectory.path)
                var settings = savedSettings()
                settings["call"] = current.call; settings["capture"] = current.input; settings["playback"] = current.output
                settings.removeValue(forKey: "hf"); settings.removeValue(forKey: "fm")
                settings[current.mode.hasPrefix("hf") ? "hf" : "fm"] = String(current.mode.dropFirst(2))
                let text = settings.keys.sorted().map { "\($0) = \(settings[$0]!)\n" }.joined()
                let settingsURL = stateDirectory.appendingPathComponent("settings")
                let temporary = stateDirectory.appendingPathComponent(".settings-" + UUID().uuidString)
                let descriptor = open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
                guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
                let file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
                defer { try? file.close(); unlink(temporary.path) }
                try file.write(contentsOf: Data(text.utf8))
                try file.synchronize()
                try file.close()
                guard rename(temporary.path, settingsURL.path) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
                defaults.set(try JSONEncoder().encode(current), forKey: "stationConfiguration")
                start()
            } catch { alert("Couldn’t save station settings", error.localizedDescription) }
        }
    }

    func start() {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: startWorker()
        case .notDetermined:
            showMessage("Microphone access", "Allow audio input so XARA can receive the signal from your selected radio interface.")
            AVCaptureDevice.requestAccess(for: .audio) { granted in DispatchQueue.main.async { [self] in
                if granted { startWorker() } else { permissionError() }
            }}
        default: permissionError()
        }
    }
    func permissionError() {
        showMessage("Audio input is unavailable", "Allow XARA in System Settings → Privacy & Security → Microphone, then choose Modem → Station & Radio to start again.")
    }
    func startWorker() {
        guard !closing else { return }
        let scripts = web.configuration.userContentController
        scripts.removeAllUserScripts()
        scripts.addUserScript(WKUserScript(source: "window.xaraNativeTransmit = \(configuration.ptt != "off"); window.xaraEmbedded = true;", injectionTime: .atDocumentStart, forMainFrameOnly: true))
        showMessage("Starting", "Core Audio")
        let result = configuration.text.withCString { config in stateDirectory.path.withCString { directory in xara_embedded_start(config, directory) } }
        guard result == 0 else { alert("Couldn’t start modem", "The modem is already starting or its configuration is invalid."); return }
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in self?.poll() }
    }
    func poll() {
        guard !closing else { return }
        var text = [CChar](repeating: 0, count: 4096)
        let state = xara_embedded_status(&text, text.count)
        let message = String(cString: text)
        engineRunning = state == 2
        if state == 2, let url = URL(string: message), url != modemURL {
            modemURL = url; web.load(URLRequest(url: url))
        } else if state == 3 {
            timer?.invalidate(); timer = nil
            showMessage("Modem error", message)
        }
    }
    func stop(_ completion: @escaping () -> Void) {
        timer?.invalidate(); timer = nil; modemURL = nil; engineRunning = false
        xara_embedded_stop()
        DispatchQueue.global(qos: .userInitiated).async {
            xara_embedded_wait()
            DispatchQueue.main.async { completion() }
        }
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        closing = true
        web.configuration.userContentController.removeScriptMessageHandler(forName: "panelReady")
        stop { sender.reply(toApplicationShouldTerminate: true) }
        return .terminateLater
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func escape(_ text: String) -> String { text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;") }
    func showMessage(_ title: String, _ detail: String) {
        web.loadHTMLString("<!doctype html><html><meta name='color-scheme' content='dark'><body style='margin:0;background:#101416;color:#e5ebea;font:12px ui-monospace,Menlo,monospace;display:grid;place-content:center;height:100vh'><main style='max-width:520px;padding:20px'><div style='color:#89d9bb;letter-spacing:5px'>XARA</div><h1 style='font-weight:500'>\(escape(title))</h1><p style='color:#84938f;line-height:1.4'>\(escape(detail))</p></main></body></html>", baseURL: nil)
    }
    func alert(_ title: String, _ detail: String) { let alert = NSAlert(); alert.messageText = title; alert.informativeText = detail; applyMonospace(alert.window.contentView); alert.runModal() }
    func applyMenuFont(_ menu: NSMenu) {
        menu.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        for item in menu.items { if let submenu = item.submenu { applyMenuFont(submenu) } }
    }
    func applyMonospace(_ view: NSView?) {
        guard let view else { return }
        if let control = view as? NSControl {
            control.font = .monospacedSystemFont(ofSize: control.font?.pointSize ?? 12, weight: .regular)
        }
        if let popup = view as? NSPopUpButton, let menu = popup.menu { applyMenuFont(menu) }
        for child in view.subviews { applyMonospace(child) }
    }
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(openInBrowser) { return engineRunning && modemURL != nil }
        if menuItem.action == #selector(fitToPanels) { return engineRunning && modemURL != nil && !window.styleMask.contains(.fullScreen) }
        return true
    }
    func windowWillStartLiveResize(_ notification: Notification) {
        // A manual resize takes precedence over the automatic startup fit.
        fitOnFirstTelemetry = false
    }
    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard message.name == "panelReady", message.frameInfo.isMainFrame,
              message.body as? String == "ready", fitOnFirstTelemetry,
              let endpoint = modemURL,
              message.frameInfo.securityOrigin.host == endpoint.host,
              message.frameInfo.securityOrigin.port == endpoint.port else { return }
        fitOnFirstTelemetry = false
        fitToPanels()
    }
    @objc func fitToPanels() {
        guard engineRunning, modemURL != nil, !window.styleMask.contains(.fullScreen),
              let screen = window.screen ?? NSScreen.main else { return }
        let visible = screen.visibleFrame.insetBy(dx: 8, dy: 8)
        let chrome = window.frame.height - window.contentLayoutRect.height
        let width = min(max(window.contentLayoutRect.width, 720), 1100, visible.width)
        if abs(window.contentLayoutRect.width - width) > 1 {
            window.setContentSize(NSSize(width: width, height: window.contentLayoutRect.height))
            web.layoutSubtreeIfNeeded()
        }
        // Measure the panels, not body.scrollHeight, which is at least the viewport height.
        web.evaluateJavaScript("(() => { const main = document.querySelector('main'); return main ? Math.ceil(main.getBoundingClientRect().bottom + window.scrollY) : null; })()") { [weak self] value, _ in
            guard let self, !self.closing, let number = value as? NSNumber else { return }
            let measured = CGFloat(number.doubleValue)
            guard measured.isFinite, measured > 0 else { return }
            var frame = self.window.frame
            let height = min(max(measured, 360 - chrome), visible.height - chrome)
            let newHeight = height + chrome
            frame.origin.y = min(max(frame.maxY - newHeight, visible.minY), visible.maxY - newHeight)
            frame.size = NSSize(width: width, height: newHeight)
            frame.origin.x = min(max(frame.minX, visible.minX), visible.maxX - frame.width)
            self.window.setFrame(frame, display: true)
        }
    }
    @objc func openInBrowser() {
        guard engineRunning, let modemURL else { return }
        if !NSWorkspace.shared.open(modemURL) {
            alert("Browser unavailable", "Could not open the local web interface.")
        }
    }
    @objc func reload() { if let modemURL { web.load(URLRequest(url: modemURL)) } }
    @objc func about() {
        NSApp.orderFrontStandardAboutPanel(options: [.applicationName: "XARA", .applicationVersion: "0.1.1 (macOS build 2)"])
    }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = action.request.url else { decisionHandler(.cancel); return }
        if url.scheme == "about", !action.shouldPerformDownload { decisionHandler(.allow); return }
        guard let endpoint = modemURL else { decisionHandler(.cancel); return }
        if action.shouldPerformDownload {
            let source = url.scheme == "blob" ? URL(string: String(url.absoluteString.dropFirst(5))) : nil
            decisionHandler(source?.scheme == endpoint.scheme && source?.host == endpoint.host && source?.port == endpoint.port ? .download : .cancel)
            return
        }
        decisionHandler(url.scheme == endpoint.scheme && url.host == endpoint.host && url.port == endpoint.port ? .allow : .cancel)
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        if (error as NSError).code != NSURLErrorCancelled { showMessage("Telemetry unavailable", "Modem → Station & Radio…") }
    }
    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) { download.delegate = self }
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String, completionHandler: @escaping (URL?) -> Void) {
        let panel = NSSavePanel(); panel.nameFieldStringValue = suggestedFilename
        applyMonospace(panel.contentView)
        panel.beginSheetModal(for: window) { result in completionHandler(result == .OK ? panel.url : nil) }
    }
}
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
