module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/plugin_query.moc"
#endif

export module waywallen:query.plugin;
export import :query.query;
export import :query.progress;

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

export class PluginInspectQuery : public Query,
                                  public QueryExtra<control::v1::Response, PluginInspectQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString zipPath READ zipPath WRITE setZipPath NOTIFY zipPathChanged FINAL)
    Q_PROPERTY(QString pluginId READ pluginId NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString name READ name NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString version READ version NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString update READ update NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool hasSource READ hasSource NOTIFY resultChanged FINAL)
    Q_PROPERTY(QStringList renderers READ renderers NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool overwrite READ overwrite NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString existingVersion READ existingVersion NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString existingName READ existingName NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool existingSystem READ existingSystem NOTIFY resultChanged FINAL)

public:
    PluginInspectQuery(QObject* parent = nullptr);

    auto zipPath() const -> const QString&;
    void setZipPath(const QString&);
    auto pluginId() const -> const QString&;
    auto name() const -> const QString&;
    auto version() const -> const QString&;
    auto update() const -> const QString&;
    auto hasSource() const -> bool;
    auto renderers() const -> const QStringList&;
    auto overwrite() const -> bool;
    auto existingVersion() const -> const QString&;
    auto existingName() const -> const QString&;
    auto existingSystem() const -> bool;

    void reload() override;

    Q_SIGNAL void zipPathChanged();
    Q_SIGNAL void resultChanged();
    Q_SIGNAL void inspected();

private:
    QString     m_zip_path;
    QString     m_plugin_id;
    QString     m_name;
    QString     m_version;
    QString     m_update;
    bool        m_has_source = false;
    QStringList m_renderers;
    bool        m_overwrite = false;
    QString     m_existing_version;
    QString     m_existing_name;
    bool        m_existing_system = false;
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

export class PluginUpdateCheckQuery
    : public ProgressQuery,
      public QueryExtra<control::v1::Response, PluginUpdateCheckQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString pluginId READ pluginId WRITE setPluginId NOTIFY pluginIdChanged FINAL)
    Q_PROPERTY(QVariantList updates READ updates NOTIFY updatesChanged FINAL)

public:
    PluginUpdateCheckQuery(QObject* parent = nullptr);

    auto pluginId() const -> const QString&;
    void setPluginId(const QString&);
    auto updates() const -> const QVariantList&;

    void             reload() override;
    Q_INVOKABLE void check(const QString& pluginId = {});

    Q_SIGNAL void pluginIdChanged();
    Q_SIGNAL void updatesChanged();
    Q_SIGNAL void checked();

private:
    QString      m_plugin_id;
    QVariantList m_updates;
};

export class PluginUpdateInstallQuery
    : public ProgressQuery,
      public QueryExtra<control::v1::Response, PluginUpdateInstallQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString pluginId READ pluginId NOTIFY pluginIdChanged FINAL)

public:
    PluginUpdateInstallQuery(QObject* parent = nullptr);

    auto pluginId() const -> const QString&;

    void             reload() override;
    Q_INVOKABLE void install(const QString& pluginId);

    Q_SIGNAL void pluginIdChanged();
    Q_SIGNAL void installed(const QString& pluginId);

private:
    QString m_plugin_id;
};

} // namespace waywallen
