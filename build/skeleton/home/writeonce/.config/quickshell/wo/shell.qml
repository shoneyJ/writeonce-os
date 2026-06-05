//@ pragma UseQApplication
//
// ~/.config/quickshell/wo/shell.qml — WriteOnce minimal Quickshell bar.
//
// A thin top bar: Hyprland workspaces (left), clock (center), volume (right).
// Imports are restricted to modules a standard (nixpkgs) Quickshell build
// always provides — NO Kirigami, QtPositioning, or Qt5Compat. Launched by
// Hyprland's `exec-once = quickshell -c wo`.

import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Services.Pipewire

ShellRoot {
    // A bar on every connected screen.
    Variants {
        model: Quickshell.screens

        PanelWindow {
            required property var modelData
            screen: modelData

            anchors { top: true; left: true; right: true }
            implicitHeight: 28
            color: "#1e1e2e"

            // ---- left: Hyprland workspaces ----
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6

                Repeater {
                    model: Hyprland.workspaces

                    Rectangle {
                        required property var modelData
                        width: 22; height: 18; radius: 4
                        color: modelData.focused ? "#0db7d4" : "#313136"

                        Text {
                            anchors.centerIn: parent
                            text: modelData.id
                            color: modelData.focused ? "#000000" : "#cdd6f4"
                            font.pixelSize: 12
                        }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: Hyprland.dispatch("workspace " + modelData.id)
                        }
                    }
                }
            }

            // ---- center: clock ----
            Text {
                id: clock
                anchors.centerIn: parent
                color: "#cdd6f4"
                font.pixelSize: 13
                font.family: "monospace"
                property string now: ""
                text: now
            }
            Timer {
                interval: 1000; running: true; repeat: true; triggeredOnStart: true
                onTriggered: clock.now = Qt.formatDateTime(new Date(), "ddd dd MMM  hh:mm")
            }

            // ---- right: default-sink volume ----
            PwObjectTracker { objects: [ Pipewire.defaultAudioSink ] }
            Text {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                color: "#cdd6f4"
                font.pixelSize: 12
                property var sink: Pipewire.defaultAudioSink
                text: {
                    if (!sink || !sink.audio) return "vol --";
                    if (sink.audio.muted)     return "vol mute";
                    return "vol " + Math.round(sink.audio.volume * 100) + "%";
                }
            }
        }
    }
}
