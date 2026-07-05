module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/progress_query.moc"
#endif

export module waywallen:query.progress;
export import :query.query;
export import :notify;

namespace waywallen
{

export class ProgressQuery : public Query {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString queryId READ queryId NOTIFY queryIdChanged FINAL)
    Q_PROPERTY(double progress READ progress NOTIFY progressChanged FINAL)
    Q_PROPERTY(bool progressing READ progressing NOTIFY progressingChanged FINAL)

public:
    ProgressQuery(QObject* parent = nullptr);

    auto queryId() const -> const QString&;
    auto progress() const -> double;
    auto progressing() const -> bool;

Q_SIGNALS:
    void queryIdChanged();
    void progressChanged();
    void progressingChanged();
    void progressEnded(bool error, const QString& message);

protected:
    void beginProgressQuery();
    void acceptProgressQuery(const QString& queryId);
    void failProgressQuery(const QString& message);

private:
    void applyTaskProgress(const TaskProgressSnapshot& snapshot);
    void setProgressValue(double progress);
    void setProgressingValue(bool progressing);

    QString m_query_id;
    double  m_progress { 0.0 };
    bool    m_progressing { false };
};

} // namespace waywallen
