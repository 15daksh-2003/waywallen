module;

#include <QGuiApplication>
#include <QList>
#include <QLocale>
#include <QString>
#include <QTranslator>
#include <QVariantList>
#include <QtQml/QQmlApplicationEngine>

export module waywallen:ui_language;
import rstd;

using rstd::boxed::Box;

export namespace waywallen
{

class UiLanguageController {
public:
    UiLanguageController(QGuiApplication& application, QQmlApplicationEngine& engine);
    ~UiLanguageController();

    auto preference() const -> const QString&;
    auto resolvedLanguage() const -> const QString&;
    auto availableLanguages() const -> QVariantList;

    auto setLanguage(const QString& preference) -> bool;
    auto refreshSystemLanguage() -> bool;

private:
    struct Language {
        QString code;
        QLocale locale;
        QString label;
    };

    void discoverLanguages();
    auto normalizePreference(const QString& preference) const -> QString;
    auto applyLanguage(const QString& preference, bool persist) -> bool;

    QGuiApplication&       m_application;
    QQmlApplicationEngine& m_engine;
    QList<Language>        m_languages;
    Box<QTranslator>       m_qt_translator;
    Box<QTranslator>       m_app_translator;
    QString                m_preference;
    QString                m_resolved_language;
    bool                   m_qt_translator_installed { false };
    bool                   m_app_translator_installed { false };
};

} // namespace waywallen
