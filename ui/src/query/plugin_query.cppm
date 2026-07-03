module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/plugin_query.moc"
#endif

export module waywallen:query.plugin;
export import :query.query;

namespace waywallen
{

// Plugin-centric (package) view: one entry per installable plugin, with the
// renderer components it provides.
export class PluginListQuery : public Query,
                               public QueryExtra<control::v1::Response, PluginListQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QVariantList plugins READ plugins NOTIFY pluginsChanged FINAL)
    Q_PROPERTY(QStringList inactiveSystem READ inactiveSystem NOTIFY pluginsChanged FINAL)
    Q_PROPERTY(QStringList inactiveUser READ inactiveUser NOTIFY pluginsChanged FINAL)

public:
    PluginListQuery(QObject* parent = nullptr);

    auto plugins() const -> const QVariantList&;
    auto inactiveSystem() const -> const QStringList&;
    auto inactiveUser() const -> const QStringList&;

    void reload() override;

    Q_SIGNAL void pluginsChanged();

private:
    QVariantList m_plugins;
    QStringList  m_inactive_system;
    QStringList  m_inactive_user;
};

export class PluginInstallQuery : public Query,
                                  public QueryExtra<control::v1::Response, PluginInstallQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString zipPath READ zipPath WRITE setZipPath NOTIFY zipPathChanged FINAL)
    Q_PROPERTY(QString pluginId READ pluginId NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool needsRestart READ needsRestart NOTIFY resultChanged FINAL)

public:
    PluginInstallQuery(QObject* parent = nullptr);

    auto zipPath() const -> const QString&;
    void setZipPath(const QString&);
    auto pluginId() const -> const QString&;
    auto needsRestart() const -> bool;

    void reload() override;

    Q_SIGNAL void zipPathChanged();
    Q_SIGNAL void resultChanged();
    Q_SIGNAL void installed(const QString& pluginId, bool needsRestart);

private:
    QString m_zip_path;
    QString m_plugin_id;
    bool    m_needs_restart = false;
};

export class PluginDeleteQuery : public Query,
                                 public QueryExtra<control::v1::Response, PluginDeleteQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString pluginId READ pluginId NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool needsRestart READ needsRestart NOTIFY resultChanged FINAL)

public:
    PluginDeleteQuery(QObject* parent = nullptr);

    auto pluginId() const -> const QString&;
    auto needsRestart() const -> bool;

    void             reload() override;
    Q_INVOKABLE void remove(const QString& pluginId);

    Q_SIGNAL void resultChanged();
    Q_SIGNAL void deleted(const QString& pluginId, bool needsRestart);

private:
    QString m_plugin_id;
    bool    m_needs_restart = false;
};

} // namespace waywallen
