import AppKit
import Darwin
let root = URL(fileURLWithPath: "/tmp/xara-profile-tests-" + UUID().uuidString, isDirectory: true)
let path = root.appendingPathComponent("profiles.json")
var hf = StationConfiguration()
hf.call = "N4EAC"; hf.input = "coreaudio:6361"; hf.output = "coreaudio:6375"; hf.ptt = "rigctld"; hf.pttAddress = "127.0.0.1:4532"
hf.externalPorts = true; hf.driveDB = 10; hf.leadMS = 50; hf.tailMS = 120
precondition(hf.hasValidValues)
var fm = hf; fm.mode = "fmwide"; fm.ptt = "off"; fm.externalPorts = false; fm.commandPort = "8400"; fm.driveDB = -12; fm.tailMS = 250
let store = RadioProfileStore(profiles: [RadioProfile(id:UUID(),name:"IC-7100 HF",station:hf),RadioProfile(id:UUID(),name:"Second radio FM",station:fm)])
try store.write(to:path)
let loaded = try RadioProfileStore.read(from:path)
precondition(loaded.profiles.count == 2)
precondition(loaded.profiles[0].station.text == hf.text && loaded.profiles[1].station.text == fm.text)
let updated = hf.merging(["drive-db":"-7","ptt-lead-ms":"60","ptt-tail-ms":"140","hf":"500","capture":"coreaudio:6362"])
precondition(updated.driveDB == -7 && updated.leadMS == 60 && updated.tailMS == 140 && updated.mode == "hf500" && updated.input == "coreaudio:6362" && updated.ptt == hf.ptt && updated.externalPorts == true)
var legacy = try JSONSerialization.jsonObject(with:JSONEncoder().encode(hf)) as! [String:Any]
for key in ["driveDB","leadMS","tailMS","externalPorts"] { legacy.removeValue(forKey:key) }
let old = try JSONDecoder().decode(StationConfiguration.self,from:JSONSerialization.data(withJSONObject:legacy))
precondition(old.driveDB == nil && old.leadMS == nil && old.tailMS == nil && old.externalPorts != true)
precondition(old.text.contains("host-ports=off") && old.text.contains("drive-db=-5.0"))
var bad = hf; bad.driveDB = 11; precondition(!bad.hasValidValues)
bad = hf; bad.input = "coreaudio:61\n62"; precondition(!bad.hasValidValues)
bad = hf; bad.leadMS = 1001; precondition(!bad.hasValidValues)
precondition(!RadioProfileStore.validName("bad\nname"))
precondition(!RadioProfileStore.validName(String(repeating:"x",count:65)))
var duplicate = store; duplicate.profiles.append(RadioProfile(id:UUID(),name:"ic-7100 hf",station:hf))
do { try duplicate.validate(); preconditionFailure("Duplicate name accepted") } catch {}
var attributes = try FileManager.default.attributesOfItem(atPath:path.path)
precondition((attributes[.posixPermissions] as! NSNumber).intValue == 0o600)
attributes = try FileManager.default.attributesOfItem(atPath:root.path)
precondition((attributes[.posixPermissions] as! NSNumber).intValue == 0o700)
let target = root.appendingPathComponent("target")
try Data("untouched".utf8).write(to:target)
try FileManager.default.removeItem(at:path)
precondition(symlink(target.path,path.path) == 0)
do { _ = try RadioProfileStore.read(from:path); preconditionFailure("Symlink read accepted") } catch {}
do { try store.write(to:path); preconditionFailure("Symlink write accepted") } catch {}
let preserved = try String(contentsOf:target,encoding:.utf8)
precondition(preserved == "untouched")
try FileManager.default.removeItem(at:path)
precondition(link(target.path,path.path) == 0)
do { try store.write(to:path); preconditionFailure("Hard link accepted") } catch {}
try FileManager.default.removeItem(at:path)
try Data("malformed".utf8).write(to:path)
do { _ = try RadioProfileStore.read(from:path); preconditionFailure("Malformed JSON accepted") } catch {}
try FileManager.default.removeItem(at:root)
print("PASS: two-radio full configuration round-trip, UI-setting merge, legacy defaults, invalid values/names, duplicates, private permissions and unsafe/malformed file rejection")

// Regression: NSPopUpButton.addItems(withTitles:) collapses duplicate titles.
// Jeremy's three same-named USB devices shifted IC-7100 onto Jump Desktop Audio.
let application = NSApplication.shared
let captureDevices = [("default", "Default"), ("iphone", "iPhone"), ("usb-a", "USB Audio CODEC"), ("usb-b", "USB Audio CODEC"), ("usb-c", "USB Audio CODEC"), ("pnp", "USB PnP Sound Device"), ("builtin", "Built-in Microphone"), ("jump-mic", "Jump Desktop Microphone"), ("jump-audio", "Jump Desktop Audio"), ("ic9700", "IC-9700, aggregate"), ("ic7100", "IC-7100, aggregate"), ("k2", "Elecraft K2, aggregate")]
let broken = NSPopUpButton(); broken.addItems(withTitles: captureDevices.map { $0.1 })
precondition(broken.numberOfItems == 10)
broken.selectItem(withTitle:"IC-7100, aggregate")
precondition(captureDevices[broken.indexOfSelectedItem].0 == "jump-audio")
let picker = AudioDevicePicker.make(captureDevices, selected:"ic7100")
precondition(picker.numberOfItems == captureDevices.count)
precondition(AudioDevicePicker.selected(picker) == "ic7100")
precondition(Set(picker.itemTitles).count == captureDevices.count)
for item in picker.itemArray {
    picker.select(item)
    let uid = AudioDevicePicker.selected(picker)!
    precondition(captureDevices.contains { $0.0 == uid })
}
let reordered = AudioDevicePicker.make(captureDevices.reversed(), selected:"usb-b")
precondition(AudioDevicePicker.selected(reordered) == "usb-b")
precondition(reordered.selectedItem?.title == "USB Audio CODEC [2]")
let missing = AudioDevicePicker.make(captureDevices, selected:"unplugged-radio")
precondition(AudioDevicePicker.selected(missing) == "unplugged-radio")
precondition(missing.selectedItem?.title == "Unavailable device")
print("PASS: reproduced original IC-7100 → Jump misrouting; all duplicate device entries retained with stable IDs, unique labels, reorder and unavailable-device preservation")
