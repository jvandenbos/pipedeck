#include "rowmodel.h"

RowModel::RowModel(const QByteArrayList &roles, const QString &key, QObject *parent)
    : QAbstractListModel(parent), m_roles(roles), m_key(key) {}

int RowModel::rowCount(const QModelIndex &parent) const {
    return parent.isValid() ? 0 : int(m_rows.size());
}

QVariant RowModel::data(const QModelIndex &index, int role) const {
    const int column = role - Qt::UserRole;
    if (!index.isValid() || index.row() >= m_rows.size() || column < 0 || column >= m_roles.size())
        return {};
    return m_rows.at(index.row()).value(QString::fromLatin1(m_roles.at(column)));
}

QHash<int, QByteArray> RowModel::roleNames() const {
    QHash<int, QByteArray> names;
    for (int i = 0; i < m_roles.size(); ++i)
        names.insert(Qt::UserRole + i, m_roles.at(i));
    return names;
}

QVariantMap RowModel::get(int row) const {
    return row >= 0 && row < m_rows.size() ? m_rows.at(row) : QVariantMap{};
}

int RowModel::indexOf(const QVariant &id) const {
    for (int i = 0; i < m_rows.size(); ++i)
        if (m_rows.at(i).value(m_key) == id)
            return i;
    return -1;
}

void RowModel::setRows(const QList<QVariantMap> &rows) {
    const int before = int(m_rows.size());

    // Drop the rows that went away, from the back so indexes stay valid.
    QList<QVariant> wanted;
    for (const auto &row : rows)
        wanted << row.value(m_key);
    for (int i = int(m_rows.size()) - 1; i >= 0; --i) {
        if (!wanted.contains(m_rows.at(i).value(m_key))) {
            beginRemoveRows({}, i, i);
            m_rows.removeAt(i);
            endRemoveRows();
        }
    }

    // Then walk the new order: update in place, move, or insert.
    for (int i = 0; i < rows.size(); ++i) {
        const QVariant id = rows.at(i).value(m_key);
        int at = -1;
        for (int j = i; j < m_rows.size(); ++j)
            if (m_rows.at(j).value(m_key) == id) {
                at = j;
                break;
            }
        if (at < 0) {
            beginInsertRows({}, i, i);
            m_rows.insert(i, rows.at(i));
            endInsertRows();
            continue;
        }
        if (at != i) {
            beginMoveRows({}, at, at, {}, i);
            m_rows.move(at, i);
            endMoveRows();
        }
        if (m_rows.at(i) != rows.at(i)) {
            QList<int> changed;
            for (int r = 0; r < m_roles.size(); ++r) {
                const QString key = QString::fromLatin1(m_roles.at(r));
                if (m_rows.at(i).value(key) != rows.at(i).value(key))
                    changed << Qt::UserRole + r;
            }
            m_rows[i] = rows.at(i);
            if (!changed.isEmpty())
                emit dataChanged(index(i), index(i), changed);
        }
    }

    if (m_rows.size() != before)
        emit countChanged();
}
