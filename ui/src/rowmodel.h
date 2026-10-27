#pragma once

#include <QAbstractListModel>
#include <QByteArrayList>
#include <QList>
#include <QVariantMap>

// A keyed list model fed whole snapshots. Rows are matched by their key
// field ("nodeId"; QML reserves "id") and updated in place, so a refresh never recreates a delegate: a
// slider being dragged, or a picker that is open, survives every snapshot.
class RowModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    explicit RowModel(const QByteArrayList &roles, const QString &key = "nodeId", QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = {}) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    void setRows(const QList<QVariantMap> &rows);
    const QList<QVariantMap> &rows() const { return m_rows; }
    Q_INVOKABLE QVariantMap get(int row) const;
    int indexOf(const QVariant &id) const;
    // One field of one row, with a dataChanged for that role only. False
    // when no row has that key.
    bool setValue(const QVariant &id, const QByteArray &role, const QVariant &value);

signals:
    void countChanged();

private:
    QByteArrayList m_roles;
    QString m_key;
    QList<QVariantMap> m_rows;
};
