#pragma once

#include <QtQml/QQmlEngine>

import waywallen;

namespace waywallen
{
class QmlRegisterHelper : public QObject {
    Q_OBJECT
    QML_ELEMENT
};
} // namespace waywallen
