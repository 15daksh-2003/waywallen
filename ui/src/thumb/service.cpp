module;
#include "waywallen/thumb/service.moc.h"

#include <algorithm>
#include <utility>

module waywallen;

import :thumb.service;
import wavsen.decode;

using namespace Qt::Literals::StringLiterals;

namespace waywallen
{

namespace
{

constexpr std::uint32_t kMaxEdge    = 512u;
constexpr int           kMaxThreads = 4;

auto thumb_root() -> QString {
    if (auto v = qEnvironmentVariable("WAYWALLEN_THUMB_DIR"); ! v.isEmpty()) {
        return v;
    }
    if (auto v = qEnvironmentVariable("XDG_CACHE_HOME"); ! v.isEmpty()) {
        return v + u"/thumbnails"_s;
    }
    return QDir::homePath() + u"/.cache/thumbnails"_s;
}

void ensure_dir(const QString& path, QFile::Permissions perms) {
    QDir().mkpath(path);
    QFile(path).setPermissions(perms);
}

auto compute_cache_path(const QString& abs_path) -> QString {
    const QString            root = thumb_root();
    const QString            sub  = root + u"/x-large"_s;
    const QFile::Permissions dir_perms { QFile::ReadOwner, QFile::WriteOwner, QFile::ExeOwner };
    ensure_dir(root, dir_perms);
    ensure_dir(sub, dir_perms);

    const QString    uri = u"file://"_s + abs_path;
    const QByteArray h   = QCryptographicHash::hash(uri.toUtf8(), QCryptographicHash::Md5).toHex();
    return sub + u"/"_s + QString::fromLatin1(h) + u".png"_s;
}

auto fit_inside(QSize src, std::uint32_t max_edge) -> QSize {
    if (src.width() <= 0 || src.height() <= 0) return QSize();
    const int me = static_cast<int>(max_edge);
    if (src.width() <= me && src.height() <= me) return src;
    if (src.width() >= src.height()) {
        return QSize(me, std::max(1, src.height() * me / src.width()));
    }
    return QSize(std::max(1, src.width() * me / src.height()), me);
}

bool write_thumb_png(const QImage& img, const QString& cache_path, const QString& uri,
                     qint64 src_mtime, qint64 src_size, QString& err_out) {
    QImage tagged = img;
    tagged.setText(u"Thumb::URI"_s, uri);
    tagged.setText(u"Thumb::MTime"_s, QString::number(src_mtime));
    tagged.setText(u"Thumb::Size"_s, QString::number(src_size));

    const auto    rnd = QRandomGenerator::system()->generate();
    const QString tmp = cache_path + u".tmp."_s +
                        QString::number(QCoreApplication::applicationPid()) + u"."_s +
                        QString::number(rnd, 16);

    QImageWriter w(tmp, "png");
    if (! w.write(tagged)) {
        err_out = w.errorString();
        QFile::remove(tmp);
        return false;
    }
    QFile(tmp).setPermissions(QFile::Permissions { QFile::ReadOwner, QFile::WriteOwner });

    // Replace atomically. QFile::rename does not overwrite on POSIX, so
    // remove the destination first if it exists.
    if (QFile::exists(cache_path)) QFile::remove(cache_path);
    if (! QFile::rename(tmp, cache_path)) {
        err_out = u"rename failed: "_s + tmp + u" -> "_s + cache_path;
        QFile::remove(tmp);
        return false;
    }
    return true;
}

} // namespace

// ---------------------------------------------------------------------------
// ThumbnailJob
// ---------------------------------------------------------------------------

ThumbnailJob::ThumbnailJob(QString key, QString cache_path, bool is_video, qint64 src_mtime,
                           qint64 src_size)
    : QObject(nullptr),
      QRunnable(),
      m_key(std::move(key)),
      m_cache_path(std::move(cache_path)),
      m_is_video(is_video),
      m_src_mtime(src_mtime),
      m_src_size(src_size) {
    // Manage lifetime via deleteLater() on the owning thread; never
    // let QThreadPool `delete this` from a worker thread on a QObject.
    setAutoDelete(false);
}

void ThumbnailJob::run() {
    QImage  img;
    QString error;

    if (m_is_video) {
        wavsen::decode::ThumbOptions opts;
        opts.max_edge = rstd::u32(kMaxEdge);
        auto res      = wavsen::decode::extract_thumbnail(m_key.toStdString(), opts);
        if (res.is_err()) {
            error = QString::fromStdString(std::move(res).unwrap_err().message);
        } else {
            auto rgba = std::move(res).unwrap();
            img       = QImage(rgba.data.data(),
                               static_cast<int>(rgba.width.to_primitive()),
                               static_cast<int>(rgba.height.to_primitive()),
                               static_cast<int>(rgba.stride.to_primitive()),
                               QImage::Format_RGBA8888)
                            .copy();
        }
    } else {
        QImageReader reader(m_key);
        reader.setAutoTransform(true);
        const QSize target = fit_inside(reader.size(), kMaxEdge);
        if (target.isValid() && ! target.isEmpty()) {
            reader.setScaledSize(target);
        }
        img = reader.read();
        if (img.isNull()) {
            error = reader.errorString();
        }
    }

    int     out_state = ThumbnailRequest::Failed;
    QString out_path;
    if (! img.isNull()) {
        const QString uri = u"file://"_s + m_key;
        QString       werr;
        if (write_thumb_png(img, m_cache_path, uri, m_src_mtime, m_src_size, werr)) {
            out_state = ThumbnailRequest::Ready;
            out_path  = m_cache_path;
        } else {
            error = werr;
        }
    }

    Q_EMIT finished(m_key, out_state, out_path, error);
}

// ---------------------------------------------------------------------------
// ThumbnailService
// ---------------------------------------------------------------------------

ThumbnailService::ThumbnailService(QObject* parent): QObject(parent) {
    m_pool.setMaxThreadCount(std::min(QThread::idealThreadCount(), kMaxThreads));
}

auto ThumbnailService::instance() -> ThumbnailService* {
    // QPointer auto-nulls when qApp tears down its child tree. Without
    // this, late-destroyed ThumbnailRequests would chase a dangling
    // pointer here and crash inside cancel() iterating freed m_pending.
    static QPointer<ThumbnailService> the = new ThumbnailService(QCoreApplication::instance());
    return the.data();
}

auto ThumbnailService::create(QQmlEngine*, QJSEngine*) -> ThumbnailService* {
    auto* s = instance();
    QJSEngine::setObjectOwnership(s, QJSEngine::CppOwnership);
    return s;
}

void ThumbnailService::submit(ThumbnailRequest* req, const QString& job_path,
                              const QString& cache_path, bool is_video, qint64 src_mtime,
                              qint64 src_size) {
    if (! req) return;
    auto it = m_pending.find(job_path);
    if (it == m_pending.end()) {
        Pending p;
        p.key        = job_path;
        p.cache_path = cache_path;
        p.subscribers.append(QPointer<ThumbnailRequest>(req));
        m_pending.insert(job_path, std::move(p));

        auto* job = new ThumbnailJob(job_path, cache_path, is_video, src_mtime, src_size);
        connect(job,
                &ThumbnailJob::finished,
                this,
                &ThumbnailService::onJobFinished,
                Qt::QueuedConnection);
        connect(job, &ThumbnailJob::finished, job, &QObject::deleteLater);
        m_pool.start(job);
    } else {
        it->subscribers.append(QPointer<ThumbnailRequest>(req));
    }
}

void ThumbnailService::cancel(ThumbnailRequest* req) {
    if (! req) return;
    QPointer<ThumbnailRequest> rp(req);
    for (auto& p : m_pending) {
        p.subscribers.removeAll(rp);
    }
}

void ThumbnailService::onJobFinished(const QString& key, int state, const QString& cache_path,
                                     const QString& error) {
    auto it = m_pending.find(key);
    if (it == m_pending.end()) return;
    auto subs = std::move(it->subscribers);
    m_pending.erase(it);

    const QUrl cache_url = cache_path.isEmpty() ? QUrl() : QUrl::fromLocalFile(cache_path);
    for (auto& wp : subs) {
        if (auto* r = wp.data()) {
            r->_applyResult(static_cast<ThumbnailRequest::State>(state), cache_url, error);
        }
    }
}

// ---------------------------------------------------------------------------
// ThumbnailRequest
// ---------------------------------------------------------------------------

ThumbnailRequest::ThumbnailRequest(QObject* parent): QObject(parent) {}

ThumbnailRequest::~ThumbnailRequest() {
    if (auto* svc = ThumbnailService::instance()) {
        svc->cancel(this);
    }
}

void ThumbnailRequest::classBegin() {}

void ThumbnailRequest::componentComplete() {
    // Wire up "re-submit on input change" only after QML's initial
    // property cascade is done. Until this point, setters merely emit
    // their *Changed signals and the lack of any subscriber keeps
    // scheduleSubmit from firing N times.
    connect(this, &ThumbnailRequest::sourceChanged, this, &ThumbnailRequest::scheduleSubmit);
    connect(this, &ThumbnailRequest::resourceChanged, this, &ThumbnailRequest::scheduleSubmit);
    connect(this, &ThumbnailRequest::wpTypeChanged, this, &ThumbnailRequest::scheduleSubmit);

    // Initial resolve — try the synchronous fast path first; if the
    // cache is hot we never enter the service at all.
    ResolvedJob rj;
    if (tryResolveSync(rj)) return;

    auto* svc = ThumbnailService::instance();
    if (! svc) return;
    setCachePathInternal(QUrl());
    setErrorInternal(QString());
    setStateInternal(Loading);
    svc->submit(this, rj.job_path, rj.cache_path, rj.is_video, rj.src_mtime, rj.src_size);
}

void ThumbnailRequest::setSource(const QString& v) {
    if (m_source == v) return;
    m_source = v;
    Q_EMIT sourceChanged();
}

void ThumbnailRequest::setResource(const QString& v) {
    if (m_resource == v) return;
    m_resource = v;
    Q_EMIT resourceChanged();
}

void ThumbnailRequest::setWpType(const QString& v) {
    if (m_wp_type == v) return;
    m_wp_type = v;
    Q_EMIT wpTypeChanged();
}

bool ThumbnailRequest::tryResolveSync(ResolvedJob& out) {
    if (! m_source.isEmpty()) {
        out.job_path = QFileInfo(m_source).absoluteFilePath();
        out.is_video = false;
    } else if (! m_resource.isEmpty() && (m_wp_type == u"video"_s || m_wp_type == u"image"_s)) {
        // No preview supplied — generate one from the resource itself.
        // Videos go through the libavformat extractor; images decode
        // via QImageReader (handled in ThumbnailJob::run by is_video).
        out.job_path = QFileInfo(m_resource).absoluteFilePath();
        out.is_video = (m_wp_type == u"video"_s);
    } else {
        setCachePathInternal(QUrl());
        setErrorInternal(u"no preview source"_s);
        setStateInternal(Failed);
        return true;
    }

    QFileInfo fi(out.job_path);
    if (! fi.exists()) {
        setCachePathInternal(QUrl());
        setErrorInternal(u"source file not found"_s);
        setStateInternal(Failed);
        return true;
    }

    out.cache_path = compute_cache_path(out.job_path);
    out.src_mtime  = fi.lastModified().toSecsSinceEpoch();
    out.src_size   = fi.size();

    // Stale-cache invalidation deferred — for now, any present cache
    // file wins. Reading the embedded `Thumb::MTime` tEXt chunk would
    // require opening + parsing the PNG, which is too slow on the GUI
    // thread for grid scrolling.
    if (QFileInfo::exists(out.cache_path)) {
        setErrorInternal(QString());
        setCachePathInternal(QUrl::fromLocalFile(out.cache_path));
        setStateInternal(Ready);
        return true;
    }
    return false;
}

void ThumbnailRequest::scheduleSubmit() {
    auto* svc = ThumbnailService::instance();
    if (! svc) return;
    svc->cancel(this);

    ResolvedJob rj;
    if (tryResolveSync(rj)) return;

    setCachePathInternal(QUrl());
    setErrorInternal(QString());
    setStateInternal(Loading);
    svc->submit(this, rj.job_path, rj.cache_path, rj.is_video, rj.src_mtime, rj.src_size);
}

void ThumbnailRequest::_applyResult(State state, QUrl cache_path, QString error) {
    setCachePathInternal(cache_path);
    setErrorInternal(error);
    setStateInternal(state);
}

void ThumbnailRequest::setStateInternal(State s) {
    if (m_state == s) return;
    m_state = s;
    Q_EMIT stateChanged();
}

void ThumbnailRequest::setCachePathInternal(const QUrl& p) {
    if (m_cache_path == p) return;
    m_cache_path = p;
    Q_EMIT cachePathChanged();
}

void ThumbnailRequest::setErrorInternal(const QString& e) {
    if (m_error == e) return;
    m_error = e;
    Q_EMIT errorChanged();
}

} // namespace waywallen

#include "waywallen/thumb/service.moc.cpp"
