module;
#include <QString>
#include <QVariant>

module waywallen;
import :proto;

namespace waywallen
{
auto runtimeConditionsFromPb(const QList<proto::RuntimeCondition>& conditions) -> QVariantList {
    QVariantList out;
    out.reserve(conditions.size());
    for (const auto& condition : conditions) {
        QString kind;
        switch (condition.kind()) {
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_LOADING:
            kind = QStringLiteral("loading");
            break;
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_WAITING:
            kind = QStringLiteral("waiting");
            break;
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_HANG:
            kind = QStringLiteral("hang");
            break;
        default: kind = QStringLiteral("issue"); break;
        }
        QString origin;
        switch (condition.origin()) {
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_RENDERER:
            origin = QStringLiteral("renderer");
            break;
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_DISPLAY:
            origin = QStringLiteral("display");
            break;
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_RELEASE:
            origin = QStringLiteral("release");
            break;
        default: origin = QStringLiteral("unknown"); break;
        }
        QVariantMap value;
        value[QStringLiteral("kind")]              = kind;
        value[QStringLiteral("origin")]            = origin;
        value[QStringLiteral("reason")]            = condition.reason();
        value[QStringLiteral("relatedRendererId")] = condition.relatedRendererId();
        value[QStringLiteral("relatedDisplayId")] =
            QVariant::fromValue<qulonglong>(condition.relatedDisplayId());
        out.append(value);
    }
    return out;
}
} // namespace waywallen
