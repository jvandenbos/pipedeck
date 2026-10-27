import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    Layout.fillWidth: true
    visible: backend.features.notifications === true
    spacing: 4

    SectionTitle {
        text: qsTr("Notifications")
        help: qsTr("Where notification sounds play: streams with the event or Notification role, and the apps listed in notification_apps. A destination picked for a stream above wins over this.")
    }

    // The radios are not checkable: the dot follows NotificationSink only,
    // so one the daemon rejected never stays selected (see DeviceEntry).
    RadioButton {
        Layout.fillWidth: true
        Layout.preferredHeight: 36
        objectName: "notify:"
        checkable: false
        text: qsTr("Follow default output")
        checked: backend.notificationSink === ""
        onClicked: if (backend.notificationSink !== "") backend.setNotificationSink("")
    }
    Repeater {
        model: backend.sinkChoices
        delegate: RadioButton {
            required property var modelData
            Layout.fillWidth: true
            Layout.preferredHeight: 36
            objectName: "notify:" + modelData.value
            checkable: false
            text: modelData.text
            checked: backend.notificationSink === modelData.value
            onClicked: if (backend.notificationSink !== modelData.value) backend.setNotificationSink(modelData.value)
        }
    }
}
