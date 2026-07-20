module;
#include "QExtra/macro_qt.hpp"
#include <QtCore/QAbstractListModel>

#ifdef Q_MOC_RUN
#    include "waywallen/model/remote_model.moc"
#endif

export module waywallen:model.remote;
export import qextra;

namespace waywallen::model
{

export struct RemoteRow {
    QString sourceId;
    QString id;
    QString title;
    QString previewUrl;
    QString author;
    QString wpType;
    int     acquisitionState { 0 };
};

export class RemoteListModel : public QAbstractListModel {
    Q_OBJECT
    QML_ANONYMOUS

    Q_PROPERTY(int count READ count NOTIFY countChanged FINAL)
    Q_PROPERTY(bool hasMore READ hasMore NOTIFY hasMoreChanged FINAL)

public:
    enum Role
    {
        ItemIdRole = Qt::UserRole + 1,
        SourceIdRole,
        TitleRole,
        PreviewUrlRole,
        AuthorRole,
        WpTypeRole,
        AcquisitionStateRole,
    };

    explicit RemoteListModel(QObject* parent = nullptr);

    int                    rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant               data(const QModelIndex& index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;
    bool                   canFetchMore(const QModelIndex& parent = QModelIndex()) const override;
    void                   fetchMore(const QModelIndex& parent = QModelIndex()) override;

    auto count() const -> int { return static_cast<int>(m_rows.size()); }
    auto hasMore() const -> bool;
    void setHasMore(bool);

    void             reset(QList<RemoteRow> rows, bool hasMore);
    void             append(const QList<RemoteRow>& rows, bool hasMore);
    Q_INVOKABLE void setAcquisitionState(const QString& sourceId, const QString& id, int state);
    Q_INVOKABLE QStringList itemIds() const;

    Q_INVOKABLE QVariantMap get(int row) const;

    Q_SIGNAL void countChanged();
    Q_SIGNAL void hasMoreChanged(bool);
    Q_SIGNAL void reqFetchMore(qint32);

private:
    QList<RemoteRow> m_rows;
    bool             m_has_more { false };
};

} // namespace waywallen::model
