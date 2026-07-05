module;
#include <algorithm>
#include "waywallen/query/progress_query.moc.h"

module waywallen;
import :query.progress;
import :app;

namespace waywallen
{

ProgressQuery::ProgressQuery(QObject* parent): Query(parent) {
    connect(
        Notify::instance(),
        &Notify::taskProgress,
        this,
        [this](const QString& queryId, double progress, bool progressing, bool ended, bool error,
               const QString& message) {
            if (m_query_id.isEmpty() || queryId != m_query_id) {
                return;
            }
            applyTaskProgress(TaskProgressSnapshot {
                .progress    = progress,
                .progressing = progressing,
                .ended       = ended,
                .error       = error,
                .message     = message,
            });
        },
        Qt::QueuedConnection);

    if (auto* backend = App::instance()->backend()) {
        connect(
            backend,
            &Backend::disconnected,
            this,
            [this] {
                if (m_progressing) {
                    failProgressQuery(QStringLiteral("backend disconnected"));
                }
            },
            Qt::QueuedConnection);
    }
}

auto ProgressQuery::queryId() const -> const QString& { return m_query_id; }
auto ProgressQuery::progress() const -> double { return m_progress; }
auto ProgressQuery::progressing() const -> bool { return m_progressing; }

void ProgressQuery::beginProgressQuery() {
    setError({});
    setProgressValue(0.0);
    setProgressingValue(true);
    setStatus(Status::Querying);
}

void ProgressQuery::acceptProgressQuery(const QString& queryId) {
    if (queryId.isEmpty()) {
        failProgressQuery(QStringLiteral("empty progress query id"));
        return;
    }
    if (m_query_id != queryId) {
        m_query_id = queryId;
        Q_EMIT queryIdChanged();
    }

    TaskProgressSnapshot snapshot;
    if (Notify::instance()->taskProgressSnapshot(m_query_id, snapshot)) {
        applyTaskProgress(snapshot);
    }
}

void ProgressQuery::failProgressQuery(const QString& message) {
    setProgressingValue(false);
    setError(message);
    setStatus(Status::Error);
    Q_EMIT progressEnded(true, message);
}

void ProgressQuery::applyTaskProgress(const TaskProgressSnapshot& snapshot) {
    setProgressValue(snapshot.progress);
    setProgressingValue(snapshot.progressing);
    if (! snapshot.ended) {
        return;
    }
    if (snapshot.error) {
        setError(snapshot.message);
        setStatus(Status::Error);
    } else {
        setStatus(Status::Finished);
    }
    Q_EMIT progressEnded(snapshot.error, snapshot.message);
}

void ProgressQuery::setProgressValue(double progress) {
    progress = std::clamp(progress, 0.0, 1.0);
    if (qFuzzyCompare(m_progress + 1.0, progress + 1.0)) {
        return;
    }
    m_progress = progress;
    Q_EMIT progressChanged();
}

void ProgressQuery::setProgressingValue(bool progressing) {
    if (m_progressing == progressing) {
        return;
    }
    m_progressing = progressing;
    Q_EMIT progressingChanged();
}

} // namespace waywallen

#include "waywallen/query/progress_query.moc.cpp"
