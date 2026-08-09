module waywallen;
import :proto;

using namespace Qt::Literals::StringLiterals;

namespace waywallen
{
auto runtimeConditionsFromPb(const QList<proto::RuntimeCondition>& conditions) -> QVariantList {
    QVariantList out;
    out.reserve(conditions.size());
    for (const auto& condition : conditions) {
        QString kind;
        switch (condition.kind()) {
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_LOADING: kind = u"loading"_s; break;
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_WAITING: kind = u"waiting"_s; break;
        case proto::RuntimeConditionKind::RUNTIME_CONDITION_HANG: kind = u"hang"_s; break;
        default: kind = u"issue"_s; break;
        }
        QString origin;
        switch (condition.origin()) {
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_RENDERER:
            origin = u"renderer"_s;
            break;
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_DISPLAY:
            origin = u"display"_s;
            break;
        case proto::RuntimeConditionOrigin::RUNTIME_CONDITION_ORIGIN_RELEASE:
            origin = u"release"_s;
            break;
        default: origin = u"unknown"_s; break;
        }
        QVariantMap value;
        value[u"kind"_s]              = kind;
        value[u"origin"_s]            = origin;
        value[u"reason"_s]            = condition.reason();
        value[u"relatedRendererId"_s] = condition.relatedRendererId();
        value[u"relatedDisplayId"_s] =
            QVariant::fromValue<qulonglong>(condition.relatedDisplayId());
        out.append(value);
    }
    return out;
}

auto runtimeTagsFromPb(const QList<proto::RendererRuntimeTag>& tags) -> QVariantList {
    QVariantList out;
    out.reserve(tags.size());
    for (const auto& tag : tags) {
        QVariantMap value;
        value[u"key"_s]   = tag.key();
        value[u"value"_s] = tag.value();
        out.append(value);
    }
    return out;
}
} // namespace waywallen
