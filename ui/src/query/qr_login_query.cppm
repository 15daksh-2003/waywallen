module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/qr_login_query.moc"
#endif

export module waywallen:query.qr_login;
export import :query.query;

namespace waywallen
{

export class QrLoginCancelQuery : public Query,
                                  public QueryExtra<control::v1::Response, QrLoginCancelQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString sessionId READ sessionId WRITE setSessionId NOTIFY sessionIdChanged FINAL)

public:
    QrLoginCancelQuery(QObject* parent = nullptr);

    auto sessionId() const -> const QString&;
    void setSessionId(const QString& value);
    void reload() override;

    Q_SIGNAL void sessionIdChanged();

private:
    QString m_session_id;
};

} // namespace waywallen
