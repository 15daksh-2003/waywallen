module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/autostart_query.moc"
#endif

export module waywallen:query.autostart;
export import :query.query;

namespace waywallen
{

export class AutostartGetQuery : public Query,
                                 public QueryExtra<control::v1::Response, AutostartGetQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(bool enabled READ enabled NOTIFY enabledChanged FINAL)

public:
    AutostartGetQuery(QObject* parent = nullptr);

    bool enabled() const;
    void reload() override;

    Q_SIGNAL void enabledChanged();

private:
    bool m_enabled = false;
};

export class AutostartSetQuery : public Query,
                                 public QueryExtra<control::v1::Response, AutostartSetQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(bool enabled READ enabled WRITE setEnabled NOTIFY enabledChanged FINAL)

public:
    AutostartSetQuery(QObject* parent = nullptr);

    bool enabled() const;
    void setEnabled(bool enabled);
    void reload() override;

    Q_SIGNAL void enabledChanged();

private:
    bool m_enabled = false;
};

} // namespace waywallen
