module;
#include "QExtra/macro_qt.hpp"
#include <algorithm>

#ifdef Q_MOC_RUN
#    include "waywallen/model/store_item.moc"
#endif

export module waywallen:model.store_item;
export import :msg.store;
export import qextra;
import rstd;
import rstd.cppstd;

export namespace waywallen::model
{

template<typename Store, typename CRTP>
class StoreItem : public QObject {
public:
    using store_type      = Store;
    using item_type       = typename store_type::item_type;
    using store_item_type = typename store_type::store_item_type;
    using handle_type     = typename store_type::handle_type;
    using key_type        = typename store_type::key_type;

    StoreItem(Store store, QObject* parent): QObject(parent), m_item(store) {}
    ~StoreItem() { unreg(); }

    auto item() const -> item_type {
        if (auto it = m_item.item()) {
            return *it;
        }
        return {};
    }
    void setItem(const item_type& v) {
        auto key = kstore::ItemTrait<item_type>::key(v);
        if (key != m_item.key()) {
            unreg();
            m_item = m_item.store().store_insert(v).first;
            m_item.store().store_changed_callback(std::span { &key, 1 },
                                                  m_handle ? *m_handle : handle_type {});
            static_cast<CRTP*>(this)->itemChanged();

            m_handle = rstd::Some(m_item.store().store_reg_notify(
                [this, key = std::move(key)](std::span<const key_type> changed) {
                    if (std::ranges::find(changed, key) != changed.end()) {
                        static_cast<CRTP*>(this)->itemChanged();
                    }
                }));
        }
    }

private:
    void unreg() {
        if (m_handle) {
            m_item.store().store_unreg_notify(*m_handle);
        }
    }

    rstd::Option<handle_type> m_handle;
    store_item_type           m_item;
};

class WallpaperStoreItem
    : public StoreItem<kstore::ItemTrait<model::Wallpaper>::store_type, WallpaperStoreItem> {
    Q_OBJECT
    QML_NAMED_ELEMENT(WallpaperStoreItem)
    Q_PROPERTY(waywallen::model::Wallpaper item READ item NOTIFY itemChanged)
public:
    using base_type =
        StoreItem<kstore::ItemTrait<model::Wallpaper>::store_type, WallpaperStoreItem>;
    WallpaperStoreItem(QObject* parent = nullptr);
    Q_SIGNAL void itemChanged();
};

class RemoteStoreItem
    : public StoreItem<kstore::ItemTrait<model::RemoteRow>::store_type, RemoteStoreItem> {
    Q_OBJECT
    QML_NAMED_ELEMENT(RemoteStoreItem)
    Q_PROPERTY(waywallen::model::RemoteRow item READ item WRITE setItem NOTIFY itemChanged)
public:
    using base_type = StoreItem<kstore::ItemTrait<model::RemoteRow>::store_type, RemoteStoreItem>;
    RemoteStoreItem(QObject* parent = nullptr);

    auto item() const -> model::RemoteRow { return base_type::item(); }
    void setItem(const model::RemoteRow& item) { base_type::setItem(item); }

    Q_SIGNAL void itemChanged();
};

} // namespace waywallen::model
