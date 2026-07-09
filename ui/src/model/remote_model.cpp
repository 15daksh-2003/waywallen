module;
#include "waywallen/model/remote_model.moc.h"

module waywallen;
import :model.remote;

using namespace Qt::Literals::StringLiterals;

namespace waywallen::model
{

RemoteListModel::RemoteListModel(QObject* parent): QAbstractListModel(parent) {}

int RemoteListModel::rowCount(const QModelIndex& parent) const {
    if (parent.isValid()) return 0;
    return static_cast<int>(m_rows.size());
}

QVariant RemoteListModel::data(const QModelIndex& index, int role) const {
    if (! index.isValid() || index.row() < 0 || index.row() >= m_rows.size()) return {};
    const auto& r = m_rows.at(index.row());
    switch (role) {
    case ItemIdRole: return r.id;
    case SourceIdRole: return r.sourceId;
    case TitleRole: return r.title;
    case PreviewUrlRole: return r.previewUrl;
    case AuthorRole: return r.author;
    case WpTypeRole: return r.wpType;
    case InstalledRole: return r.installed;
    default: return {};
    }
}

QHash<int, QByteArray> RemoteListModel::roleNames() const {
    return {
        { ItemIdRole, "itemId"_ba },       { SourceIdRole, "sourceId"_ba },
        { TitleRole, "title"_ba },         { PreviewUrlRole, "previewUrl"_ba },
        { AuthorRole, "author"_ba },       { WpTypeRole, "wpType"_ba },
        { InstalledRole, "installed"_ba },
    };
}

bool RemoteListModel::canFetchMore(const QModelIndex& parent) const {
    return ! parent.isValid() && m_has_more;
}

void RemoteListModel::fetchMore(const QModelIndex& parent) {
    if (parent.isValid() || ! m_has_more) return;
    setHasMore(false);
    Q_EMIT reqFetchMore(rowCount());
}

auto RemoteListModel::hasMore() const -> bool { return m_has_more; }

void RemoteListModel::setHasMore(bool v) {
    if (m_has_more != v) {
        m_has_more = v;
        Q_EMIT hasMoreChanged(v);
    }
}

void RemoteListModel::reset(QList<RemoteRow> rows) {
    beginResetModel();
    m_rows = std::move(rows);
    endResetModel();
    Q_EMIT countChanged();
}

void RemoteListModel::append(const QList<RemoteRow>& rows) {
    if (rows.isEmpty()) return;
    const int first = static_cast<int>(m_rows.size());
    beginInsertRows(QModelIndex(), first, first + static_cast<int>(rows.size()) - 1);
    m_rows.append(rows);
    endInsertRows();
    Q_EMIT countChanged();
}

void RemoteListModel::setInstalled(const QString& sourceId, const QString& id, bool installed) {
    for (int i = 0; i < m_rows.size(); ++i) {
        if (m_rows.at(i).sourceId == sourceId && m_rows.at(i).id == id) {
            if (m_rows[i].installed != installed) {
                m_rows[i].installed = installed;
                const auto idx      = index(i, 0);
                Q_EMIT dataChanged(idx, idx);
            }
            return;
        }
    }
}

QVariantMap RemoteListModel::get(int row) const {
    QVariantMap m;
    if (row < 0 || row >= m_rows.size()) return m;
    const auto& r      = m_rows.at(row);
    m["sourceId"_L1]   = r.sourceId;
    m["itemId"_L1]     = r.id;
    m["title"_L1]      = r.title;
    m["previewUrl"_L1] = r.previewUrl;
    m["author"_L1]     = r.author;
    m["wpType"_L1]     = r.wpType;
    m["installed"_L1]  = r.installed;
    return m;
}

} // namespace waywallen::model

#include "waywallen/model/remote_model.moc.cpp"
