module;
#include <QtCore/QDebug>

module waywallen;
import :ui_language;

using namespace Qt::Literals::StringLiterals;
using namespace rstd::prelude;

namespace
{
constexpr auto kSystemLanguage  = "system";
constexpr auto kEnglishLanguage = "en";
constexpr auto kSettingsKey     = "ui/language";

auto locale_label(const QLocale& locale, const QString& fallback) -> QString {
    auto language = locale.nativeLanguageName();
    if (language.isEmpty()) language = fallback;

    const auto territory = locale.nativeTerritoryName();
    if (territory.isEmpty()) return language;
    return u"%1 (%2)"_s.arg(language, territory);
}
} // namespace

namespace waywallen
{

UiLanguageController::UiLanguageController(QGuiApplication&       application,
                                           QQmlApplicationEngine& engine)
    : m_application(application),
      m_engine(engine),
      m_qt_translator(Box<QTranslator>::make()),
      m_app_translator(Box<QTranslator>::make()) {
    discoverLanguages();

    QSettings  settings;
    const auto saved      = settings.value(kSettingsKey, kSystemLanguage).toString();
    auto       preference = normalizePreference(saved);
    if (preference.isEmpty()) {
        qWarning("ui language: unsupported saved preference '%s', falling back to system",
                 qPrintable(saved));
        preference = QString::fromLatin1(kSystemLanguage);
    }

    if (! applyLanguage(preference, false)) {
        qWarning("ui language: failed to load '%s', falling back to system",
                 qPrintable(preference));
        if (! applyLanguage(QString::fromLatin1(kSystemLanguage), false)) {
            qCritical("ui language: failed to initialize system language");
        }
    }

    if (saved != m_preference) settings.setValue(kSettingsKey, m_preference);
}

UiLanguageController::~UiLanguageController() {
    if (m_app_translator_installed) m_application.removeTranslator(m_app_translator.as_mut_ptr());
    if (m_qt_translator_installed) m_application.removeTranslator(m_qt_translator.as_mut_ptr());
}

auto UiLanguageController::preference() const -> const QString& { return m_preference; }

auto UiLanguageController::resolvedLanguage() const -> const QString& {
    return m_resolved_language;
}

auto UiLanguageController::availableLanguages() const -> QVariantList {
    QVariantList result;
    result.reserve(m_languages.size() + 2);

    QVariantMap system;
    system.insert(u"code"_s, QString::fromLatin1(kSystemLanguage));
    system.insert(u"label"_s, QCoreApplication::translate("SettingsPage", "System"));
    result.push_back(system);

    QVariantMap english;
    english.insert(u"code"_s, QString::fromLatin1(kEnglishLanguage));
    english.insert(u"label"_s, QStringLiteral("English"));
    result.push_back(english);

    for (const auto& language : m_languages) {
        QVariantMap value;
        value.insert(u"code"_s, language.code);
        value.insert(u"label"_s, language.label);
        result.push_back(value);
    }
    return result;
}

auto UiLanguageController::setLanguage(const QString& preference) -> bool {
    const auto normalized = normalizePreference(preference);
    if (normalized.isEmpty()) {
        qWarning("ui language: rejected unsupported preference '%s'", qPrintable(preference));
        return false;
    }
    if (normalized == m_preference) return true;
    return applyLanguage(normalized, true);
}

auto UiLanguageController::refreshSystemLanguage() -> bool {
    if (m_preference != QString::fromLatin1(kSystemLanguage)) return true;
    return applyLanguage(m_preference, false);
}

void UiLanguageController::discoverLanguages() {
    const QDir i18n_dir(QStringLiteral(":/i18n"));
    const auto catalogs =
        i18n_dir.entryList({ QStringLiteral("waywallen_*.qm") }, QDir::Files, QDir::Name);
    constexpr auto prefix_size = qsizetype(sizeof("waywallen_") - 1);
    constexpr auto suffix_size = qsizetype(sizeof(".qm") - 1);

    for (const auto& catalog : catalogs) {
        const auto code = catalog.sliced(prefix_size, catalog.size() - prefix_size - suffix_size);
        const QLocale locale(code);
        if (locale.language() == QLocale::C || locale.language() == QLocale::English) continue;

        bool duplicate = false;
        for (const auto& language : m_languages) {
            if (language.code == code) {
                duplicate = true;
                break;
            }
        }
        if (duplicate) continue;

        m_languages.push_back(Language {
            .code   = code,
            .locale = locale,
            .label  = locale_label(locale, code),
        });
    }
}

auto UiLanguageController::normalizePreference(const QString& preference) const -> QString {
    if (preference == QString::fromLatin1(kSystemLanguage)) return preference;
    if (preference == QString::fromLatin1(kEnglishLanguage)) return preference;

    for (const auto& language : m_languages) {
        if (language.code == preference) return language.code;
    }

    const QLocale requested(preference);
    if (requested.language() == QLocale::English) return QString::fromLatin1(kEnglishLanguage);
    if (requested.language() == QLocale::C) return {};

    for (const auto& language : m_languages) {
        if (language.locale.name() == requested.name()) return language.code;
    }
    return {};
}

auto UiLanguageController::applyLanguage(const QString& preference, bool persist) -> bool {
    const auto locale = preference == QString::fromLatin1(kSystemLanguage) ? QLocale::system()
                                                                           : QLocale(preference);

    // "qt" is the umbrella catalog and ships only with a full Qt installation.
    // Deployments that carry qtbase alone - the AppImage, the Flatpak runtime,
    // distros that split Qt translations per module - have "qtbase" only, so
    // fall back to it before giving up on Qt's own strings.
    const auto qt_translations = QLibraryInfo::path(QLibraryInfo::TranslationsPath);

    auto next_qt_translator = Box<QTranslator>::make();
    auto next_qt_catalog    = QStringLiteral("qt");
    bool next_qt_loaded =
        next_qt_translator->load(locale, next_qt_catalog, QStringLiteral("_"), qt_translations);
    if (! next_qt_loaded) {
        next_qt_catalog = QStringLiteral("qtbase");
        next_qt_loaded =
            next_qt_translator->load(locale, next_qt_catalog, QStringLiteral("_"), qt_translations);
    }

    auto       next_app_translator = Box<QTranslator>::make();
    const bool next_app_loaded     = next_app_translator->load(
        locale, QStringLiteral("waywallen"), QStringLiteral("_"), QStringLiteral(":/i18n"));
    const bool app_catalog_required = preference != QString::fromLatin1(kSystemLanguage) &&
                                      preference != QString::fromLatin1(kEnglishLanguage);
    if (app_catalog_required && ! next_app_loaded) {
        qWarning("ui language: application catalog for '%s' could not be loaded",
                 qPrintable(preference));
        return false;
    }

    if (m_app_translator_installed) m_application.removeTranslator(m_app_translator.as_mut_ptr());
    if (m_qt_translator_installed) m_application.removeTranslator(m_qt_translator.as_mut_ptr());

    const bool next_qt_installed =
        next_qt_loaded && m_application.installTranslator(next_qt_translator.as_mut_ptr());
    const bool next_app_installed =
        next_app_loaded && m_application.installTranslator(next_app_translator.as_mut_ptr());
    if ((next_qt_loaded && ! next_qt_installed) || (next_app_loaded && ! next_app_installed)) {
        if (next_app_installed) m_application.removeTranslator(next_app_translator.as_mut_ptr());
        if (next_qt_installed) m_application.removeTranslator(next_qt_translator.as_mut_ptr());
        if (m_qt_translator_installed)
            (void)m_application.installTranslator(m_qt_translator.as_mut_ptr());
        if (m_app_translator_installed)
            (void)m_application.installTranslator(m_app_translator.as_mut_ptr());
        qWarning("ui language: failed to install translators for '%s'", qPrintable(preference));
        return false;
    }

    m_qt_translator            = rstd::move(next_qt_translator);
    m_app_translator           = rstd::move(next_app_translator);
    m_qt_translator_installed  = next_qt_installed;
    m_app_translator_installed = next_app_installed;
    m_preference               = preference;
    m_resolved_language =
        locale.language() == QLocale::C ? QString::fromLatin1(kEnglishLanguage) : locale.name();

    m_engine.setUiLanguage(locale.bcp47Name());
    m_engine.retranslate();

    if (persist) {
        QSettings settings;
        settings.setValue(kSettingsKey, m_preference);
    }

    qInfo("ui language: preference=%s resolved=%s app_catalog=%s qt_catalog=%s",
          qPrintable(m_preference),
          qPrintable(m_resolved_language),
          next_app_loaded ? "loaded" : "source",
          next_qt_loaded ? qPrintable(next_qt_catalog) : "source");
    return true;
}

} // namespace waywallen
