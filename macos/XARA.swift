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
    var driveDB: Double?
    var leadMS: Int?
    var tailMS: Int?

    var text: String {
        let modeLine = mode.hasPrefix("hf") ? "hf=\(mode.dropFirst(2))" : "fm=\(mode.dropFirst(2))"
        var method = ptt
        if ["rts", "dtr", "rigctld", "cm108"].contains(ptt) { method += ":" + pttAddress }
        return "call=\(call)\n\(modeLine)\ncapture=\(input)\nplayback=\(output)\nptt=\(method)\nhost-ports=\(externalPorts == true ? "on" : "off")\nport=\(commandPort)\nkiss-port=\(kissPort)\ndrive-db=\(driveDB ?? -5)\nptt-lead-ms=\(leadMS ?? 0)\nptt-tail-ms=\(tailMS ?? 100)"
    }
}

struct RadioProfile: Codable {
    let id: UUID
    var name: String
    var station: StationConfiguration
}

struct RadioProfileStore: Codable {
    var schema = 1
    var profiles = [RadioProfile]()
    static let maximumBytes = 262_144

    static func validName(_ name: String) -> Bool {
        !name.isEmpty && name.count <= 64 && name == name.trimmingCharacters(in: .whitespacesAndNewlines) && !name.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
    }
    func validate() throws {
        guard schema == 1, profiles.count <= 32,
              profiles.allSatisfy({ Self.validName($0.name) && $0.station.hasValidValues }),
              Set(profiles.map { $0.name.lowercased() }).count == profiles.count,
              Set(profiles.map { $0.id }).count == profiles.count else { throw failure("Invalid or unsupported profile file.") }
    }
    static func read(from url: URL) throws -> Self {
        var directory = stat()
        let parent = url.deletingLastPathComponent()
        if lstat(parent.path, &directory) != 0 {
            if errno == ENOENT { return Self() }
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        guard directory.st_mode & S_IFMT == S_IFDIR, directory.st_uid == geteuid() else { throw failure("Unsafe profile directory.") }
        let descriptor = open(url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        if descriptor < 0 {
            if errno == ENOENT { return Self() }
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        let file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? file.close() }
        var metadata = stat()
        guard fstat(descriptor, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFREG,
              metadata.st_nlink == 1, metadata.st_uid == geteuid(), metadata.st_size <= maximumBytes else { throw failure("Unsafe or oversized profile file.") }
        guard fchmod(descriptor, 0o600) == 0 else { throw POSIXError(.EACCES) }
        let data = try file.read(upToCount: maximumBytes + 1) ?? Data()
        guard data.count <= maximumBytes else { throw failure("Profile file is too large.") }
        let store = try JSONDecoder().decode(Self.self, from: data)
        try store.validate()
        return store
    }
    func write(to url: URL) throws {
        try validate()
        let directory = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var metadata = stat()
        guard lstat(directory.path, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFDIR, metadata.st_uid == geteuid() else { throw failure("Unsafe profile directory.") }
        guard chmod(directory.path, 0o700) == 0 else { throw POSIXError(.EACCES) }
        if lstat(url.path, &metadata) == 0 {
            guard metadata.st_mode & S_IFMT == S_IFREG, metadata.st_nlink == 1, metadata.st_uid == geteuid() else { throw failure("Unsafe profile file.") }
        } else if errno != ENOENT { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let data = try JSONEncoder().encode(self)
        guard data.count <= Self.maximumBytes else { throw failure("Profile file is too large.") }
        let temporary = directory.appendingPathComponent(".profiles-" + UUID().uuidString)
        let descriptor = open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? file.close(); unlink(temporary.path) }
        try file.write(contentsOf: data); try file.synchronize(); try file.close()
        guard rename(temporary.path, url.path) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }
    static func failure(_ text: String) -> NSError { NSError(domain: "XARA.Profiles", code: 1, userInfo: [NSLocalizedDescriptionKey: text]) }
    private func failure(_ text: String) -> NSError { Self.failure(text) }
}

extension StationConfiguration {
    var hasValidValues: Bool {
        let validCall = !call.utf8.contains(0) && call.withCString { xara_embedded_valid_call($0) != 0 }
        let command = Int(commandPort) ?? -1, kiss = Int(kissPort) ?? -1
        let validKiss = kissPort == "off" || (1...65535).contains(kiss) && kiss != command && kiss != command + 1
        let validDevice: (String) -> Bool = { value in
            if value == "default" { return true }
            guard value.hasPrefix("coreaudio:") else { return false }
            let hex = value.dropFirst(10)
            return !hex.isEmpty && hex.count % 2 == 0 && hex.count <= 2048 && hex.utf8.allSatisfy { (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }
        }
        let safe = [input, output, pttAddress].allSatisfy { !$0.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) && $0.utf8.count <= 2048 }
        return validCall && validDevice(input) && validDevice(output) && ["hf500", "hf2300", "hf2750", "fmnarrow", "fmwide"].contains(mode) && ["off", "vox", "rts", "dtr", "rigctld", "cm108"].contains(ptt) && safe && !input.isEmpty && !output.isEmpty && (1...65534).contains(command) && validKiss && (driveDB ?? -5).isFinite && (-30...10).contains(driveDB ?? -5) && (0...1000).contains(leadMS ?? 0) && (0...1000).contains(tailMS ?? 100) && (ptt == "off" || (input != "default" && output != "default"))
    }
    func merging(_ saved: [String: String]) -> Self {
        var result = self
        result.call = saved["call"] ?? call; result.input = saved["capture"] ?? input; result.output = saved["playback"] ?? output
        if let value = saved["hf"] { result.mode = "hf" + value }
        if let value = saved["fm"] { result.mode = "fm" + value }
        result.driveDB = saved["drive-db"].flatMap(Double.init) ?? driveDB ?? -5
        result.leadMS = saved["ptt-lead-ms"].flatMap(Int.init) ?? leadMS ?? 0
        result.tailMS = saved["ptt-tail-ms"].flatMap(Int.init) ?? tailMS ?? 100
        return result
    }
}

struct AudioDevicePicker {
    static func make(_ devices: [(String, String)], selected: String) -> NSPopUpButton {
        var choices = devices
        if !choices.contains(where: { $0.0 == selected }) { choices.append((selected, "Unavailable device")) }
        let groups = Dictionary(grouping: choices, by: { $0.1 })
        let popup = NSPopUpButton()
        let menu = NSMenu(); menu.autoenablesItems = false
        for (uid, description) in choices {
            let duplicates = (groups[description] ?? []).map { $0.0 }.sorted()
            let title = duplicates.count > 1 ? "\(description) [\((duplicates.firstIndex(of: uid) ?? 0) + 1)]" : description
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            item.representedObject = uid; item.toolTip = uid
            menu.addItem(item)
        }
        popup.menu = menu
        if let item = menu.items.first(where: { $0.representedObject as? String == selected }) { popup.select(item) }
        return popup
    }
    static func selected(_ popup: NSPopUpButton) -> String? { popup.selectedItem?.representedObject as? String }
}

struct MacReleaseVersion: Equatable, Comparable {
    let components: [Int]
    let build: Int

    init?(version: String, build: String) {
        let parts = version.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 3, parts.allSatisfy({ !$0.isEmpty && $0.allSatisfy({ $0.isASCII && $0.isNumber }) }),
              !build.isEmpty, build.allSatisfy({ $0.isASCII && $0.isNumber }),
              let build = Int(build), build > 0 else { return nil }
        let numbers = parts.compactMap { Int($0) }
        guard numbers.count == 3 else { return nil }
        self.components = numbers; self.build = build
    }

    init?(tag: String) {
        let pieces = tag.split(separator: "-", omittingEmptySubsequences: false)
        guard pieces.count == 2, pieces[0].hasPrefix("v"), pieces[1].hasPrefix("macos.") else { return nil }
        self.init(version: String(pieces[0].dropFirst()), build: String(pieces[1].dropFirst(6)))
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        if lhs.components != rhs.components { return lhs.components.lexicographicallyPrecedes(rhs.components) }
        return lhs.build < rhs.build
    }

    var description: String { components.map(String.init).joined(separator: ".") + " (macOS build \(build))" }
}

struct MacReleaseCheck {
    static let releasesURL = URL(string: "https://github.com/N4EAC/xara-for-macos/releases")!
    static let latestAPI = URL(string: "https://api.github.com/repos/N4EAC/xara-for-macos/releases/latest")!

    struct Release {
        let tag: String
        let version: MacReleaseVersion
        var url: URL { URL(string: "https://github.com/N4EAC/xara-for-macos/releases/tag/" + tag)! }
    }

    static func decode(_ data: Data) throws -> Release {
        guard data.count <= 1_048_576,
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["draft"] as? Bool == false, object["prerelease"] as? Bool == false,
              let tag = object["tag_name"] as? String, let version = MacReleaseVersion(tag: tag) else {
            throw NSError(domain: "XARA.Update", code: 1, userInfo: [NSLocalizedDescriptionKey: "GitHub did not return a supported public macOS release."])
        }
        return Release(tag: tag, version: version)
    }

    static func latest(_ completion: @escaping (Result<Release, Error>) -> Void) {
        let config = URLSessionConfiguration.ephemeral
        config.urlCache = nil; config.httpCookieStorage = nil; config.urlCredentialStorage = nil
        config.timeoutIntervalForRequest = 15; config.timeoutIntervalForResource = 20
        let session = URLSession(configuration: config)
        var request = URLRequest(url: latestAPI)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue("XARA-macOS", forHTTPHeaderField: "User-Agent")
        session.dataTask(with: request) { data, response, error in
            defer { session.finishTasksAndInvalidate() }
            let result = Result<Release, Error> {
                if let error { throw error }
                guard let response = response as? HTTPURLResponse, response.statusCode == 200,
                      response.url?.scheme == "https", response.url?.host == "api.github.com",
                      response.url?.path.lowercased() == latestAPI.path.lowercased(), let data else {
                    throw NSError(domain: "XARA.Update", code: 2, userInfo: [NSLocalizedDescriptionKey: "Couldn’t read the latest N4EAC GitHub release. Check your Internet connection or open GitHub Releases."])
                }
                return try decode(data)
            }
            DispatchQueue.main.async { completion(result) }
        }.resume()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKDownloadDelegate, NSMenuItemValidation, WKScriptMessageHandler, NSWindowDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var modemURL: URL?
    var engineRunning = false
    var fitOnFirstTelemetry = true
    var timer: Timer?
    var checkingRelease = false
    var closing = false
    var diagnosticInitError = ""
    var configuration = StationConfiguration()
    let defaults = UserDefaults.standard
    var stateDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("XARA", isDirectory: true)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let logs = stateDirectory.appendingPathComponent("Diagnostics", isDirectory: true)
        var error = [CChar](repeating: 0, count: 4096)
        logs.path.withCString { _ = xara_diagnostics_init($0, &error, error.count) }
        diagnosticInitError = String(cString: error)
        let info = Bundle.main.infoDictionary ?? [:]
        diagnostic("app_start version=\(info["CFBundleShortVersionString"] ?? "unknown") build=\(info["CFBundleVersion"] ?? "unknown") os=\(ProcessInfo.processInfo.operatingSystemVersionString) architecture=arm64")
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
        modemMenu.addItem(withTitle: "Export Diagnostic Log…", action: #selector(exportDiagnosticLog), keyEquivalent: "")
        modemMenu.addItem(withTitle: "Diagnostic Logging…", action: #selector(diagnosticLoggingInfo), keyEquivalent: "")
        modemMenu.addItem(withTitle: "Reload", action: #selector(reload), keyEquivalent: "r")
        modemMenu.addItem(withTitle: "Fit to Panels", action: #selector(fitToPanels), keyEquivalent: "0")
        let browserItem = modemMenu.addItem(withTitle: "Open in Browser", action: #selector(openInBrowser), keyEquivalent: "b")
        browserItem.target = self
        browserItem.keyEquivalentModifierMask = [.command, .shift]
        let profilesItem = NSMenuItem(); menu.addItem(profilesItem)
        let profilesMenu = NSMenu(title: "Profiles"); profilesItem.submenu = profilesMenu
        profilesMenu.addItem(withTitle: "Save Current Profile…", action: #selector(saveRadioProfile), keyEquivalent: "")
        profilesMenu.addItem(withTitle: "Load or Delete Profile…", action: #selector(manageRadioProfiles), keyEquivalent: "")
        let updateItem = NSMenuItem(); menu.addItem(updateItem)
        let updateMenu = NSMenu(title: "Update"); updateItem.submenu = updateMenu
        updateMenu.addItem(withTitle: "Check for Updates…", action: #selector(checkForUpdates), keyEquivalent: "")
        updateMenu.addItem(withTitle: "GitHub Releases…", action: #selector(showReleases), keyEquivalent: "")
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
        diagnostic("device_refresh direction=\(kind)")
        let count = xara_embedded_devices(nil, 0)
        var text = [CChar](repeating: 0, count: max(count, 1))
        let needed = xara_embedded_devices(&text, text.count)
        guard needed <= text.count,
              let data = String(cString: text).data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let list = object[kind] as? [[String: String]] else { return [] }
        return list.compactMap { d in guard let name = d["name"], let desc = d["desc"] else { return nil }; return (name, desc) }
    }

    var profileURL: URL { stateDirectory.appendingPathComponent("profiles.json") }
    @objc func saveRadioProfile() {
        let current = configuration.merging(savedSettings())
        guard current.hasValidValues else { alert("Configure station first", "Choose valid station and radio settings before saving a profile."); return }
        do {
            var store = try RadioProfileStore.read(from: profileURL)
            let dialog = NSAlert(); dialog.messageText = "Save current radio profile"
            dialog.informativeText = "Saves station, mode, audio device IDs, PTT, external ports, TX drive and timing locally. Audio and received messages are excluded."
            let name = NSTextField(string: ""); name.placeholderString = "Profile name, for example IC-7100 HF"
            name.frame = NSRect(x: 0, y: 0, width: 360, height: 28); dialog.accessoryView = name
            dialog.addButton(withTitle: "Save"); dialog.addButton(withTitle: "Cancel")
            applyMonospace(dialog.window.contentView); dialog.window.initialFirstResponder = name
            guard dialog.runModal() == .alertFirstButtonReturn else { return }
            let title = name.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard RadioProfileStore.validName(title) else { alert("Invalid profile name", "Use 1–64 characters without control characters."); return }
            if let index = store.profiles.firstIndex(where: { $0.name.lowercased() == title.lowercased() }) {
                let confirm = NSAlert(); confirm.messageText = "Replace profile “\(store.profiles[index].name)”?"
                confirm.addButton(withTitle: "Replace"); confirm.addButton(withTitle: "Cancel"); applyMonospace(confirm.window.contentView)
                guard confirm.runModal() == .alertFirstButtonReturn else { return }
                store.profiles[index].name = title; store.profiles[index].station = current
            } else {
                guard store.profiles.count < 32 else { alert("Profile limit reached", "Delete an unused profile before adding another. Up to 32 profiles are supported."); return }
                store.profiles.append(RadioProfile(id: UUID(), name: title, station: current))
            }
            try store.write(to: profileURL)
            diagnostic("profile_saved count=\(store.profiles.count)")
        } catch { alert("Couldn’t save profile", error.localizedDescription) }
    }
    @objc func manageRadioProfiles() {
        do {
            var store = try RadioProfileStore.read(from: profileURL)
            store.profiles.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            guard !store.profiles.isEmpty else { alert("No saved profiles", "Use Profiles → Save Current Profile after configuring a radio."); return }
            let dialog = NSAlert(); dialog.messageText = "Radio profiles"
            dialog.informativeText = "Load opens Station & Radio for review. Start modem applies the profile while idle. Missing devices remain selected as unavailable. Deleting a profile does not change the running modem."
            let selection = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 360, height: 28))
            selection.addItems(withTitles: store.profiles.map { $0.name }); dialog.accessoryView = selection
            dialog.addButton(withTitle: "Load profile…"); dialog.addButton(withTitle: "Delete"); dialog.addButton(withTitle: "Close")
            applyMonospace(dialog.window.contentView)
            let result = dialog.runModal()
            let profile = store.profiles[selection.indexOfSelectedItem]
            if result == .alertFirstButtonReturn {
                diagnostic("profile_load_requested")
                configureStation(profile.station)
            } else if result == .alertSecondButtonReturn {
                let confirm = NSAlert(); confirm.messageText = "Delete profile “\(profile.name)”?"
                confirm.addButton(withTitle: "Delete"); confirm.addButton(withTitle: "Cancel"); applyMonospace(confirm.window.contentView)
                if confirm.runModal() == .alertFirstButtonReturn {
                    store.profiles.removeAll { $0.id == profile.id }; try store.write(to: profileURL)
                    diagnostic("profile_deleted count=\(store.profiles.count)")
                }
            }
        } catch { alert("Couldn’t load profiles", error.localizedDescription) }
    }

    @objc func configure() { configureStation(configuration.merging(savedSettings())) }
    func configureStation(_ selected: StationConfiguration) {
        guard xara_embedded_can_configure() != 0 else { alert("Modem is busy", "Disconnect the radio link and stop tuning before changing station hardware."); return }
        var current = selected
        let dialog = NSAlert()
        dialog.messageText = "Station & Radio"
        let form = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 560))
        var y: CGFloat = 532
        func add(_ title: String, _ control: NSControl) {
            let label = NSTextField(labelWithString: title)
            label.frame = NSRect(x: 0, y: y + 4, width: 140, height: 20)
            control.frame = NSRect(x: 145, y: y, width: 375, height: 28)
            control.setAccessibilityLabel(title)
            form.addSubview(label); form.addSubview(control); y -= 35
        }
        let call = NSTextField(string: current.call); call.placeholderString = "Callsign"; add("Callsign", call)
        let mode = NSPopUpButton()
        let modes = [("hf500", "HF · 500 Hz"), ("hf2300", "HF · 2300 Hz"), ("hf2750", "HF · 2750 Hz"), ("fmnarrow", "FM · Narrow"), ("fmwide", "FM · Wide")]
        mode.addItems(withTitles: modes.map { $0.1 }); mode.selectItem(at: modes.firstIndex { $0.0 == current.mode } ?? 1); add("Modem mode", mode)
        func deviceMenu(_ kind: String, _ selected: String) -> NSPopUpButton {
            AudioDevicePicker.make([("default", "System default (receive only)")] + deviceChoices(kind), selected: selected)
        }
        let input = deviceMenu("capture", current.input); add("Radio audio input", input)
        let output = deviceMenu("playback", current.output); add("Radio audio output", output)
        let ptt = NSPopUpButton()
        let methods = [("off", "Receive only · transmit disabled"), ("rts", "Serial RTS"), ("dtr", "Serial DTR"), ("rigctld", "Hamlib rigctld"), ("cm108", "CM108 USB HID"), ("vox", "VOX")]
        ptt.addItems(withTitles: methods.map { $0.1 }); ptt.selectItem(at: methods.firstIndex { $0.0 == current.ptt } ?? 0); add("PTT method", ptt)
        let address = NSTextField(string: current.pttAddress); address.placeholderString = "Serial path, rigctld host:port, or CM108 VID:PID[:GPIO]"; add("PTT device / host", address)
        let ports = NSButton(checkboxWithTitle: "Enable command/data and KISS ports", target: nil, action: nil)
        ports.state = current.externalPorts == true ? .on : .off
        add("External software", ports)
        let port = NSTextField(string: current.commandPort); add("Command port", port)
        let kiss = NSTextField(string: current.kissPort); kiss.placeholderString = "8100 or off"; add("KISS port", kiss)
        let drive = NSTextField(string: String(current.driveDB ?? -5)); add("TX drive · dB", drive)
        let lead = NSTextField(string: String(current.leadMS ?? 0)); add("PTT lead · ms", lead)
        let tail = NSTextField(string: String(current.tailMS ?? 100)); add("PTT tail · ms", tail)
        let note = NSTextField(wrappingLabelWithString: "Data port = command port + 1. Ports are local and unauthenticated; enable only for trusted radio software.\nSaved locally: station, audio, PTT, and port settings. TX drive is remembered automatically. Received data stays in memory unless exported.")
        note.font = .monospacedSystemFont(ofSize: 11, weight: .regular); note.textColor = .secondaryLabelColor
        note.frame = NSRect(x: 0, y: 0, width: 520, height: 74); form.addSubview(note)
        dialog.accessoryView = form
        dialog.addButton(withTitle: "Start modem"); dialog.addButton(withTitle: "Cancel")
        applyMonospace(dialog.window.contentView)
        dialog.window.initialFirstResponder = call
        guard dialog.runModal() == .alertFirstButtonReturn else { return }
        current.call = call.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        current.mode = modes[mode.indexOfSelectedItem].0
        guard let capture = AudioDevicePicker.selected(input), let playback = AudioDevicePicker.selected(output) else {
            alert("Check audio selection", "Select an input and output device before applying settings."); return
        }
        current.input = capture; current.output = playback
        current.ptt = methods[ptt.indexOfSelectedItem].0
        current.pttAddress = address.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        current.externalPorts = ports.state == .on
        current.commandPort = port.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        current.kissPort = kiss.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        current.driveDB = Double(drive.stringValue); current.leadMS = Int(lead.stringValue); current.tailMS = Int(tail.stringValue)
        let validCall = !current.call.utf8.contains(0) && current.call.withCString { xara_embedded_valid_call($0) != 0 }
        let command = Int(current.commandPort) ?? -1
        let kissNumber = Int(current.kissPort) ?? -1
        let validKiss = current.kissPort == "off" || (1...65535).contains(kissNumber) && kissNumber != command && kissNumber != command + 1
        guard current.driveDB != nil, current.leadMS != nil, current.tailMS != nil, current.hasValidValues, validCall, (1...65534).contains(command), validKiss,
              !current.pttAddress.contains(where: { $0.isNewline }),
              current.ptt == "off" || (current.input != "default" && current.output != "default") else {
            alert("Check station settings", "Enter a valid callsign, distinct ports, explicit radio devices for transmit, TX drive −30 to +10 dB, and PTT lead/tail 0–1000 ms."); return
        }
        guard xara_embedded_can_configure() != 0 else {
            alert("Modem is busy", "A radio operation started while settings were open. Disconnect and stop tuning before applying changes."); return
        }
        stop { [self] in
            do {
                try FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let attributes = try FileManager.default.attributesOfItem(atPath: stateDirectory.path)
                guard attributes[.type] as? FileAttributeType == .typeDirectory else { throw POSIXError(.EINVAL) }
                try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: stateDirectory.path)
                var settings = savedSettings()
                settings["drive-db"] = String(current.driveDB ?? -5); settings["ptt-lead-ms"] = String(current.leadMS ?? 0); settings["ptt-tail-ms"] = String(current.tailMS ?? 100)
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
                configuration = current
                defaults.set(try JSONEncoder().encode(current), forKey: "stationConfiguration")
                start()
            } catch { alert("Couldn’t save station settings", error.localizedDescription) }
        }
    }

    func start() {
        diagnostic("microphone_permission status=\(AVCaptureDevice.authorizationStatus(for: .audio).rawValue)")
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: startWorker()
        case .notDetermined:
            showMessage("Microphone access", "Allow audio input so XARA can receive the signal from your selected radio interface.")
            AVCaptureDevice.requestAccess(for: .audio) { granted in DispatchQueue.main.async { [self] in
                diagnostic("microphone_permission_response granted=\(granted)")
                if granted { startWorker() } else { permissionError() }
            }}
        default: permissionError()
        }
    }
    func permissionError() {
        diagnostic("microphone_permission_denied")
        showMessage("Audio input is unavailable", "Allow XARA in System Settings → Privacy & Security → Microphone, then choose Modem → Station & Radio to start again.")
    }
    func startWorker() {
        guard !closing else { return }
        let scripts = web.configuration.userContentController
        scripts.removeAllUserScripts()
        scripts.addUserScript(WKUserScript(source: "window.xaraNativeTransmit = \(configuration.ptt != "off"); window.xaraEmbedded = true;", injectionTime: .atDocumentStart, forMainFrameOnly: true))
        showMessage("Starting", "Core Audio")
        diagnostic("engine_start_requested")
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
        diagnostic("engine_stop_requested")
        timer?.invalidate(); timer = nil; modemURL = nil; engineRunning = false
        xara_embedded_stop()
        DispatchQueue.global(qos: .userInitiated).async {
            xara_embedded_wait()
            _ = xara_diagnostics_export(nil, 0) // Bounded writer barrier before normal shutdown/reconfiguration.
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
        web.loadHTMLString("<!doctype html><html><meta name='color-scheme' content='dark'><body style='margin:0;background:#101416;color:#e5ebea;font:12px ui-monospace,Menlo,monospace;display:grid;place-content:center;height:100vh'><main style='max-width:520px;padding:20px'><div style='color:#89d9bb;letter-spacing:5px'>XARA</div><h1 style='font-weight:500'>\(escape(title))</h1><p style='color:#84938f;line-height:1.4'>\(escape(detail))</p><p style='color:#84938f;line-height:1.4'>Automatic diagnostic logs: up to 2 MiB locally. No audio or message contents. Modem → Diagnostic Logging / Export Diagnostic Log.</p></main></body></html>", baseURL: nil)
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
        if menuItem.action == #selector(checkForUpdates) { return !checkingRelease }
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
    @objc func showReleases() {
        NSWorkspace.shared.open(MacReleaseCheck.releasesURL)
    }

    @objc func checkForUpdates() {
        guard !checkingRelease else { return }
        guard let info = Bundle.main.infoDictionary,
              let version = info["CFBundleShortVersionString"] as? String,
              let build = info["CFBundleVersion"] as? String,
              let installed = MacReleaseVersion(version: version, build: build) else {
            alert("Couldn’t check updates", "The installed app has no valid version/build information."); return
        }
        checkingRelease = true
        MacReleaseCheck.latest { [weak self] result in
            guard let self else { return }
            self.checkingRelease = false
            guard !self.closing else { return }
            switch result {
            case .failure(let error): self.alert("Couldn’t check updates", error.localizedDescription)
            case .success(let release):
                if release.version == installed {
                    self.alert("Check for Updates", "Xara UI for Mac by N4EAC is up-to-date.")
                } else if release.version < installed {
                    self.alert("Check for Updates", "No newer GitHub release is available.\nInstalled: \(installed.description)\nLatest published: \(release.version.description)")
                } else {
                    let panel = NSAlert()
                    panel.messageText = "A newer Xara UI for Mac is available"
                    panel.informativeText = "Installed: \(installed.description)\nGitHub: \(release.version.description)"
                    panel.addButton(withTitle: "Open GitHub Release")
                    panel.addButton(withTitle: "Later")
                    self.applyMonospace(panel.window.contentView)
                    if panel.runModal() == .alertFirstButtonReturn { NSWorkspace.shared.open(release.url) }
                }
            }
        }
    }

    func diagnostic(_ text: String) { text.withCString { xara_diagnostics_event($0) } }
    @objc func diagnosticLoggingInfo() {
        let status = diagnosticInitError.isEmpty ? "Automatic operational logging is enabled." : "Logging failed: " + diagnosticInitError
        alert("Diagnostic logging", "\(status)\n\nCurrent and previous log: up to 1 MiB each, replaced automatically as they fill and at launch. Stored in:\n\(stateDirectory.appendingPathComponent("Diagnostics").path)\n\nIncludes OS/build, audio device names and IDs, channels, rates, latency, errors and numeric signal/timing summaries. No audio recordings, message contents, raw host commands or authentication tokens. Nothing is uploaded automatically. Use Export Diagnostic Log to share it deliberately.")
    }
    @objc func exportDiagnosticLog() {
        let panel = NSSavePanel(); panel.nameFieldStringValue = "XARA-diagnostics.txt"
        applyMonospace(panel.contentView)
        panel.beginSheetModal(for: window) { [weak self] result in
            guard result == .OK, let destination = panel.url, let self else { return }
            let initializationError = self.diagnosticInitError
            DispatchQueue.global(qos: .utility).async {
                // Export is bounded by two 1 MiB logs plus a small metadata header.
                var text = [CChar](repeating: 0, count: 2_200_000)
                let needed = xara_diagnostics_export(&text, text.count)
                do {
                    guard needed <= text.count else { throw NSError(domain: "XARA.Diagnostics", code: 1, userInfo: [NSLocalizedDescriptionKey: "Diagnostic export exceeded its size limit."]) }
                    let report = initializationError.isEmpty ? String(cString: text) : "Logging initialization failed: \(initializationError)\n" + String(cString: text)
                    try report.write(to: destination, atomically: true, encoding: .utf8)
                } catch { DispatchQueue.main.async { self.alert("Couldn’t export diagnostics", error.localizedDescription) } }
            }
        }
    }

    func aboutPanel() -> NSAlert {
        let panel = NSAlert()
        panel.messageText = "Xara modem for Mac"
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "0.1.3"
        let build = info["CFBundleVersion"] as? String ?? "8"
        panel.informativeText = "By N4EAC, Eduardo\nXARA \(version) · macOS build \(build)\nOriginal modem by x0rro"
        panel.addButton(withTitle: "GitHub Releases")
        panel.addButton(withTitle: "Original XARA")
        panel.addButton(withTitle: "Close")
        applyMonospace(panel.window.contentView)
        return panel
    }
    @objc func about() {
        let panel = aboutPanel()
        let result = panel.runModal()
        if result == .alertFirstButtonReturn { showReleases() }
        if result == .alertSecondButtonReturn { NSWorkspace.shared.open(URL(string: "https://github.com/x0rro/xara")!) }
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
