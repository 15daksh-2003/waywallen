module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/global_pause_query.moc"
#endif

export module waywallen:query.global_pause;
export import :query.query;

namespace waywallen
{

export class GlobalPauseToggleQuery
    : public Query,
      public QueryExtra<control::v1::Response, GlobalPauseToggleQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(bool paused READ paused NOTIFY pausedChanged FINAL)

public:
    GlobalPauseToggleQuery(QObject* parent = nullptr);

    bool paused() const;
    void reload() override;

    Q_SIGNAL void pausedChanged();
    Q_SIGNAL void toggled(bool paused);

private:
    bool m_paused = false;
};

} // namespace waywallen
