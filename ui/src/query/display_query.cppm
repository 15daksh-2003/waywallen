module;
#include "QExtra/macro_qt.hpp"
#include <optional>

#ifdef Q_MOC_RUN
#    include "waywallen/query/display_query.moc"
#endif

export module waywallen:query.display;
export import :query.query;

namespace waywallen
{

export class DisplayListQuery : public Query,
                                public QueryExtra<control::v1::Response, DisplayListQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QVariantList displays READ displays NOTIFY displaysChanged FINAL)

public:
    DisplayListQuery(QObject* parent = nullptr);

    auto displays() const -> const QVariantList&;

    void reload() override;

    Q_SIGNAL void displaysChanged();

private:
    QVariantList m_displays;
};

/// Mutate a single display's per-display layout override. Set
/// `fillmodeSet` (true) + `fillmode` (int FillMode enum) to write a
/// fillmode override; `clearFillmode = true` removes the override
/// (revert to global default). Same pattern for `location*`,
/// `align*`, and `rotation*`. Empty `name` is rejected by the daemon. The daemon re-emits
/// composition config to the live consumer and broadcasts a
/// `DisplayChanged` event with the refreshed `effectiveLayout`.
///
/// Clear color is NOT exposed here — it's owned by the renderer.
export class DisplayLayoutSetQuery
    : public Query,
      public QueryExtra<control::v1::Response, DisplayLayoutSetQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString name READ name WRITE setName NOTIFY paramsChanged FINAL)
    Q_PROPERTY(quint64 displayId READ displayId WRITE setDisplayId NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool fillmodeSet READ fillmodeSet WRITE setFillmodeSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int fillmode READ fillmode WRITE setFillmode NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool locationSet READ locationSet WRITE setLocationSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int locationX READ locationX WRITE setLocationX NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int locationY READ locationY WRITE setLocationY NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool alignSet READ alignSet WRITE setAlignSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int align READ align WRITE setAlign NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool rotationSet READ rotationSet WRITE setRotationSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int rotation READ rotation WRITE setRotation NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearFillmode READ clearFillmode WRITE setClearFillmode NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearLocation READ clearLocation WRITE setClearLocation NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool clearAlign READ clearAlign WRITE setClearAlign NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearRotation READ clearRotation WRITE setClearRotation NOTIFY paramsChanged FINAL)

public:
    DisplayLayoutSetQuery(QObject* parent = nullptr);

    auto name() const -> const QString& { return m_name; }
    void setName(const QString& v);
    auto displayId() const -> quint64 { return m_display_id; }
    void setDisplayId(quint64 v);
    auto fillmodeSet() const -> bool { return m_fillmode_set; }
    void setFillmodeSet(bool v);
    auto fillmode() const -> int { return m_fillmode; }
    void setFillmode(int v);
    auto locationSet() const -> bool { return m_location_set; }
    void setLocationSet(bool v);
    auto locationX() const -> int { return m_location_x; }
    void setLocationX(int v);
    auto locationY() const -> int { return m_location_y; }
    void setLocationY(int v);
    auto alignSet() const -> bool { return m_align_set; }
    void setAlignSet(bool v);
    auto align() const -> int { return m_align; }
    void setAlign(int v);
    auto rotationSet() const -> bool { return m_rotation_set; }
    void setRotationSet(bool v);
    auto rotation() const -> int { return m_rotation; }
    void setRotation(int v);
    auto clearFillmode() const -> bool { return m_clear_fillmode; }
    void setClearFillmode(bool v);
    auto clearLocation() const -> bool { return m_clear_location; }
    void setClearLocation(bool v);
    auto clearAlign() const -> bool { return m_clear_align; }
    void setClearAlign(bool v);
    auto clearRotation() const -> bool { return m_clear_rotation; }
    void setClearRotation(bool v);

    void reload() override;

    Q_SIGNAL void paramsChanged();

private:
    QString m_name;
    quint64 m_display_id { 0 };
    bool    m_fillmode_set { false };
    int     m_fillmode { 0 };
    bool    m_location_set { false };
    int     m_location_x { 50 };
    int     m_location_y { 50 };
    bool    m_align_set { false };
    int     m_align { 0 };
    bool    m_rotation_set { false };
    int     m_rotation { 0 };
    bool    m_clear_fillmode { false };
    bool    m_clear_location { false };
    bool    m_clear_align { false };
    bool    m_clear_rotation { false };
};

export class CanvasLayoutSetQuery : public Query,
                                    public QueryExtra<control::v1::Response, CanvasLayoutSetQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString canvasId READ canvasId WRITE setCanvasId NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool fillmodeSet READ fillmodeSet WRITE setFillmodeSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int fillmode READ fillmode WRITE setFillmode NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool locationSet READ locationSet WRITE setLocationSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int locationX READ locationX WRITE setLocationX NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int locationY READ locationY WRITE setLocationY NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool rotationSet READ rotationSet WRITE setRotationSet NOTIFY paramsChanged FINAL)
    Q_PROPERTY(int rotation READ rotation WRITE setRotation NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearFillmode READ clearFillmode WRITE setClearFillmode NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearLocation READ clearLocation WRITE setClearLocation NOTIFY paramsChanged FINAL)
    Q_PROPERTY(
        bool clearRotation READ clearRotation WRITE setClearRotation NOTIFY paramsChanged FINAL)

public:
    CanvasLayoutSetQuery(QObject* parent = nullptr);

    auto canvasId() const -> const QString& { return m_canvas_id; }
    void setCanvasId(const QString& v);
    auto fillmodeSet() const -> bool { return m_fillmode_set; }
    void setFillmodeSet(bool v);
    auto fillmode() const -> int { return m_fillmode; }
    void setFillmode(int v);
    auto locationSet() const -> bool { return m_location_set; }
    void setLocationSet(bool v);
    auto locationX() const -> int { return m_location_x; }
    void setLocationX(int v);
    auto locationY() const -> int { return m_location_y; }
    void setLocationY(int v);
    auto rotationSet() const -> bool { return m_rotation_set; }
    void setRotationSet(bool v);
    auto rotation() const -> int { return m_rotation; }
    void setRotation(int v);
    auto clearFillmode() const -> bool { return m_clear_fillmode; }
    void setClearFillmode(bool v);
    auto clearLocation() const -> bool { return m_clear_location; }
    void setClearLocation(bool v);
    auto clearRotation() const -> bool { return m_clear_rotation; }
    void setClearRotation(bool v);

    void reload() override;

    Q_SIGNAL void paramsChanged();

private:
    QString m_canvas_id;
    bool    m_fillmode_set { false };
    int     m_fillmode { 0 };
    bool    m_location_set { false };
    int     m_location_x { 50 };
    int     m_location_y { 50 };
    bool    m_rotation_set { false };
    int     m_rotation { 0 };
    bool    m_clear_fillmode { false };
    bool    m_clear_location { false };
    bool    m_clear_rotation { false };
};

export class DisplayRenameQuery : public Query,
                                  public QueryExtra<control::v1::Response, DisplayRenameQuery> {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QString name READ name WRITE setName NOTIFY paramsChanged FINAL)
    Q_PROPERTY(quint64 displayId READ displayId WRITE setDisplayId NOTIFY paramsChanged FINAL)
    Q_PROPERTY(QString alias READ alias WRITE setAlias NOTIFY paramsChanged FINAL)
    Q_PROPERTY(bool clear READ clear WRITE setClear NOTIFY paramsChanged FINAL)

public:
    DisplayRenameQuery(QObject* parent = nullptr);

    auto name() const -> const QString& { return m_name; }
    void setName(const QString& v);
    auto displayId() const -> quint64 { return m_display_id; }
    void setDisplayId(quint64 v);
    auto alias() const -> const QString& { return m_alias; }
    void setAlias(const QString& v);
    auto clear() const -> bool { return m_clear; }
    void setClear(bool v);

    void reload() override;

    Q_SIGNAL void paramsChanged();

private:
    QString m_name;
    quint64 m_display_id { 0 };
    QString m_alias;
    bool    m_clear { false };
};

export class CanvasListQuery : public Query,
                               public QueryExtra<control::v1::Response, CanvasListQuery> {
    Q_OBJECT
    QML_ELEMENT

public:
    CanvasListQuery(QObject* parent = nullptr);

    void reload() override;
};

export class CanvasMutationQuery : public Query,
                                   public QueryExtra<control::v1::Response, CanvasMutationQuery> {
    Q_OBJECT
    QML_ELEMENT

public:
    CanvasMutationQuery(QObject* parent = nullptr);

    Q_INVOKABLE void createCanvas(const QString& name, const QVariantList& members);
    Q_INVOKABLE void updateCanvas(const QString& id, quint64 expectedRevision, const QString& name,
                                  const QVariantList& members);
    Q_INVOKABLE void removeCanvas(const QString& id, quint64 expectedRevision);
    void             reload() override;

    Q_SIGNAL void canvasCreated(const QString& id);
    Q_SIGNAL void canvasUpdated(const QString& id, quint64 revision);
    Q_SIGNAL void canvasDeleted(const QString& id);

private:
    std::optional<control::v1::Request> m_request;
};

} // namespace waywallen
