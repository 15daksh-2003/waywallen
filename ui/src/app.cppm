module;
#ifdef Q_MOC_RUN
#    include "waywallen/app.moc"
#endif

#include <QEvent>
#include <QVariantList>
#include "QExtra/macro_qt.hpp"

export module waywallen:app;
export import :backend;
export import :display;
export import :gpu;
export import :renderer;
export import :library;
export import qextra;

class AppPrivate;

namespace waywallen
{
export class App : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    Q_PROPERTY(DisplayManager* displayManager READ displayManager CONSTANT FINAL)
    Q_PROPERTY(RendererManager* rendererManager READ rendererManager CONSTANT FINAL)
    Q_PROPERTY(LibraryManager* libraryManager READ libraryManager CONSTANT FINAL)
    Q_PROPERTY(GpuManager* gpuManager READ gpuManager CONSTANT FINAL)
    Q_PROPERTY(qint64 networkCacheSize READ networkCacheSize NOTIFY networkCacheSizeChanged FINAL)
    Q_PROPERTY(qint64 networkCacheMaximumSize READ networkCacheMaximumSize NOTIFY
                   networkCacheMaximumSizeChanged FINAL)
    Q_PROPERTY(QString uiLanguage READ uiLanguage NOTIFY uiLanguageChanged FINAL)
    Q_PROPERTY(
        QString resolvedUiLanguage READ resolvedUiLanguage NOTIFY resolvedUiLanguageChanged FINAL)
    Q_PROPERTY(
        QVariantList availableUiLanguages READ availableUiLanguages NOTIFY uiLanguageChanged FINAL)

public:
    App(quint16 port, rstd::empty);
    virtual ~App();
    static App* create(QQmlEngine* qmlEngine, QJSEngine* jsEngine);

    // make qml prefer create
    App() = delete;

    void init();

    static auto instance() -> App*;

    auto displayManager() const -> DisplayManager*;
    auto rendererManager() const -> RendererManager*;
    auto libraryManager() const -> LibraryManager*;
    auto gpuManager() const -> GpuManager*;
    auto networkCacheSize() const -> qint64;
    auto networkCacheMaximumSize() const -> qint64;
    auto uiLanguage() const -> const QString&;
    auto resolvedUiLanguage() const -> const QString&;
    auto availableUiLanguages() const -> QVariantList;

    auto engine() const -> QQmlApplicationEngine*;
    auto backend() const -> Backend*;

    Q_SLOT void      load_settings();
    Q_SLOT void      save_settings();
    Q_SLOT void      refreshNetworkCacheSize();
    Q_SLOT void      setNetworkCacheMaximumSize(qint64 size);
    Q_SLOT void      clearNetworkCache();
    Q_INVOKABLE bool setUiLanguage(const QString& language);

    Q_SIGNAL void errorOccurred(const QString& error);
    Q_SIGNAL void networkCacheSizeChanged();
    Q_SIGNAL void networkCacheMaximumSizeChanged();
    Q_SIGNAL void uiLanguageChanged();
    Q_SIGNAL void resolvedUiLanguageChanged();

protected:
    bool eventFilter(QObject* watched, QEvent* event) override;

private:
    QScopedPointer<AppPrivate> d_ptr;
    Q_DECLARE_PRIVATE(App);
};
} // namespace waywallen
