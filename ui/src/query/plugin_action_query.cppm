module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/plugin_action_query.moc"
#endif

export module waywallen:query.plugin_action;
export import :query.query;

namespace waywallen
{

// Fire-and-forget trigger for a plugin-declared action. Set pluginId/actionId,
// then reload(). The daemon routes it (e.g. workshop Steam sign-in/out) and any
// progress arrives via its own events (e.g. Notify::steamLoginProgress).
export class PluginActionQuery : public Query,
                                 public QueryExtra<control::v1::Response, PluginActionQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString pluginId READ pluginId WRITE setPluginId NOTIFY pluginIdChanged FINAL)
    Q_PROPERTY(QString actionId READ actionId WRITE setActionId NOTIFY actionIdChanged FINAL)

public:
    PluginActionQuery(QObject* parent = nullptr);

    QString pluginId() const;
    void    setPluginId(const QString& v);
    QString actionId() const;
    void    setActionId(const QString& v);
    void    reload() override;

    Q_SIGNAL void pluginIdChanged();
    Q_SIGNAL void actionIdChanged();

private:
    QString m_plugin_id;
    QString m_action_id;
};

} // namespace waywallen
