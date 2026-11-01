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
