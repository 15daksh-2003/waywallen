module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/health_query.moc"
#endif

export module waywallen:query.health;
export import :query.query;

namespace waywallen
{

export class HealthQuery : public Query, public QueryExtra<control::v1::Response, HealthQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString service READ service NOTIFY serviceChanged FINAL)
    Q_PROPERTY(QString state READ state NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString osName READ osName NOTIFY osNameChanged FINAL)

public:
    HealthQuery(QObject* parent = nullptr);

    auto service() const -> const QString&;
    auto state() const -> const QString&;
    auto osName() const -> const QString&;

    void reload() override;

    Q_SIGNAL void serviceChanged();
    Q_SIGNAL void stateChanged();
    Q_SIGNAL void osNameChanged();

private:
    QString m_service;
    QString m_state;
    QString m_os_name;
};

} // namespace waywallen
