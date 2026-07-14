module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/steam_login_query.moc"
#endif

export module waywallen:query.steam_login;
export import :query.query;

namespace waywallen
{

// Fire-and-forget: the daemon drives DepotDownloader's QR login and reports
// progress via Notify::steamLoginProgress, so these queries only send.
export class SteamLoginStartQuery : public Query,
                                    public QueryExtra<control::v1::Response, SteamLoginStartQuery> {
    Q_OBJECT
    QML_ELEMENT

public:
    SteamLoginStartQuery(QObject* parent = nullptr);
    void reload() override;
};

export class SteamLoginCancelQuery
    : public Query,
      public QueryExtra<control::v1::Response, SteamLoginCancelQuery> {
    Q_OBJECT
    QML_ELEMENT

public:
    SteamLoginCancelQuery(QObject* parent = nullptr);
    void reload() override;
};

} // namespace waywallen
