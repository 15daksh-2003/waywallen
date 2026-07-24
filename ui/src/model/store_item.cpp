module;
#include "waywallen/model/store_item.moc.h"

module waywallen;
import :model.store_item;
import :msg.store;

namespace waywallen::model
{

WallpaperStoreItem::WallpaperStoreItem(QObject* parent)
    : base_type(AppStore::instance()->wallpapers, parent) {}

RemoteStoreItem::RemoteStoreItem(QObject* parent)
    : base_type(AppStore::instance()->remotes, parent) {}

} // namespace waywallen::model

#include "waywallen/model/store_item.moc.cpp"
