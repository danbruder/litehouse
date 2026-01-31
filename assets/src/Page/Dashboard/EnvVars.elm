module Page.Dashboard.EnvVars exposing
    ( Model
    , Msg(..)
    , init
    , update
    , view
    )

{-| Environment variables management for app detail page.
-}

import Effect exposing (Effect)
import Html exposing (Html, button, div, input, label, p, span, text)
import Html.Attributes exposing (class, disabled, for, id, placeholder, required, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)


-- MODEL


type alias Model =
    { envVars : List Effect.EnvVar
    , envVarsLoading : Bool
    , envVarForm : FormState
    }


type alias FormState =
    { key : String
    , value : String
    , editingKey : Maybe String
    }


init : Model
init =
    { envVars = []
    , envVarsLoading = False
    , envVarForm =
        { key = ""
        , value = ""
        , editingKey = Nothing
        }
    }


-- UPDATE


type Msg
    = FetchEnvVars
    | GotEnvVars (Result String (List Effect.EnvVar))
    | EnvVarKeyChanged String
    | EnvVarValueChanged String
    | SubmitEnvVar
    | CancelEnvVarEdit
    | EditEnvVar String
    | DeleteEnvVar String
    | GotEnvVarSet (Result String String)


update : Msg -> Model -> String -> String -> ( Model, Effect Msg, Maybe String )
update msg model token appName =
    case msg of
        FetchEnvVars ->
            ( { model | envVarsLoading = True }
            , Effect.FetchEnvVars token appName GotEnvVars
            , Nothing
            )

        GotEnvVars result ->
            case result of
                Ok envVars ->
                    ( { model
                        | envVars = envVars
                        , envVarsLoading = False
                      }
                    , Effect.none
                    , Nothing
                    )

                Err _ ->
                    ( { model | envVarsLoading = False }
                    , Effect.none
                    , Nothing
                    )

        EnvVarKeyChanged key ->
            let
                currentForm =
                    model.envVarForm

                updatedForm =
                    { currentForm | key = key }
            in
            ( { model | envVarForm = updatedForm }
            , Effect.none
            , Nothing
            )

        EnvVarValueChanged newValue ->
            let
                currentForm =
                    model.envVarForm

                updatedForm =
                    { currentForm | value = newValue }
            in
            ( { model | envVarForm = updatedForm }
            , Effect.none
            , Nothing
            )

        SubmitEnvVar ->
            let
                formState =
                    model.envVarForm
            in
            if String.isEmpty (String.trim formState.key) then
                ( model
                , Effect.none
                , Just "Environment variable key is required"
                )

            else
                ( { model
                    | envVarForm = { key = "", value = "", editingKey = Nothing }
                  }
                , Effect.SetEnvVar token appName formState.key formState.value False GotEnvVarSet
                , Nothing
                )

        CancelEnvVarEdit ->
            ( { model
                | envVarForm = { key = "", value = "", editingKey = Nothing }
              }
            , Effect.none
            , Nothing
            )

        EditEnvVar key ->
            let
                envVar =
                    List.filter (\ev -> ev.key == key) model.envVars
                        |> List.head
            in
            case envVar of
                Just ev ->
                    ( { model
                        | envVarForm =
                            { key = ev.key
                            , value = ev.value
                            , editingKey = Just ev.key
                            }
                      }
                    , Effect.none
                    , Nothing
                    )

                Nothing ->
                    ( model, Effect.none, Nothing )

        DeleteEnvVar key ->
            ( { model
                | envVarForm = { key = "", value = "", editingKey = Nothing }
              }
            , Effect.SetEnvVar token appName key "" True GotEnvVarSet
            , Nothing
            )

        GotEnvVarSet result ->
            case result of
                Ok _ ->
                    ( model
                    , Effect.FetchEnvVars token appName GotEnvVars
                    , Nothing
                    )

                Err err ->
                    ( model
                    , Effect.none
                    , Just err
                    )


-- VIEW


view : Model -> Html Msg
view model =
    div [ class "space-y-4" ]
        [ if model.envVarsLoading then
            div [ class "flex items-center justify-center gap-3 py-4 text-litehouse-muted" ]
                [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                , span [] [ text "Loading environment variables..." ]
                ]

          else
            div []
                [ -- List of existing env vars
                  if List.isEmpty model.envVars then
                    p [ class "text-sm text-litehouse-muted italic mb-4" ] [ text "No environment variables set" ]

                  else
                    div [ class "mb-4 space-y-2" ]
                        (List.map (viewEnvVarRow model) model.envVars)

                -- Form to add/edit env var
                , viewEnvVarForm model
                ]
        ]


viewEnvVarRow : Model -> Effect.EnvVar -> Html Msg
viewEnvVarRow model envVar =
    let
        isEditing =
            model.envVarForm.editingKey == Just envVar.key
    in
    if isEditing then
        text ""

    else
        div [ class "flex items-center justify-between p-3 bg-litehouse-bg rounded-xl border border-litehouse-border" ]
            [ div [ class "flex-1" ]
                [ div [ class "text-sm font-medium text-litehouse-text font-mono" ] [ text envVar.key ]
                , div [ class "text-xs text-litehouse-muted font-mono mt-1 truncate" ] [ text envVar.value ]
                ]
            , div [ class "flex items-center gap-2 ml-4" ]
                [ button
                    [ class "px-3 py-1.5 text-xs border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                    , onClick (EditEnvVar envVar.key)
                    ]
                    [ text "Edit" ]
                , button
                    [ class "px-3 py-1.5 text-xs bg-litehouse-error hover:bg-litehouse-error/80 text-white rounded-xl transition-colors"
                    , onClick (DeleteEnvVar envVar.key)
                    ]
                    [ text "Delete" ]
                ]
            ]


viewEnvVarForm : Model -> Html Msg
viewEnvVarForm model =
    let
        formState =
            model.envVarForm

        isEditing =
            formState.editingKey /= Nothing

        submitLabel =
            if isEditing then
                "Update"

            else
                "Add"

        cancelButton =
            if isEditing then
                button
                    [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                    , onClick CancelEnvVarEdit
                    ]
                    [ text "Cancel" ]

            else
                text ""
    in
    Html.form [ onSubmit SubmitEnvVar, class "space-y-3" ]
        [ div [ class "grid grid-cols-2 gap-3" ]
            [ div []
                [ label [ for "envKey", class "block mb-1 text-xs font-medium text-litehouse-text" ] [ text "Key" ]
                , input
                    [ type_ "text"
                    , id "envKey"
                    , value formState.key
                    , onInput EnvVarKeyChanged
                    , placeholder "ENV_VAR_NAME"
                    , required True
                    , disabled isEditing
                    , class "w-full px-3 py-2 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:opacity-50 disabled:cursor-not-allowed font-mono"
                    ]
                    []
                ]
            , div []
                [ label [ for "envValue", class "block mb-1 text-xs font-medium text-litehouse-text" ] [ text "Value" ]
                , input
                    [ type_ "text"
                    , id "envValue"
                    , value formState.value
                    , onInput EnvVarValueChanged
                    , placeholder "value"
                    , required True
                    , class "w-full px-3 py-2 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 font-mono"
                    ]
                    []
                ]
            ]
        , div [ class "flex items-center gap-3" ]
            [ button
                [ type_ "submit"
                , class "px-4 py-2 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                ]
                [ text submitLabel ]
            , cancelButton
            ]
        ]
